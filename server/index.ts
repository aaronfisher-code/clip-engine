import { createHash, randomUUID } from "node:crypto";
import { readdir, rename, stat } from "node:fs/promises";
import path from "node:path";
import express from "express";
import multer from "multer";
import { config, r2Configured } from "./config.js";
import { deleteManagedClipFiles } from "./library.js";
import { makePreview, probeClip } from "./media.js";
import { createPublishJob } from "./publisher.js";
import { Store } from "./store.js";
import type { Clip } from "./types.js";

const mediaExtensions = new Set([".mkv", ".mp4", ".mov", ".webm", ".avi", ".m4v"]);
const store = new Store();
await store.init();

const app = express();
app.disable("x-powered-by");
app.use(express.json({ limit: "1mb" }));

const upload = multer({
  dest: config.uploadDir,
  limits: { fileSize: 100 * 1024 * 1024 * 1024, files: 20 },
});

function publicClip(clip: Clip) {
  const { sourcePath: _sourcePath, fingerprint: _fingerprint, previewPath: _previewPath, ...safe } = clip;
  return safe;
}

let previewQueue = Promise.resolve();

function queuePreview(clip: Clip) {
  if (clip.previewStatus === "processing" || clip.previewStatus === "ready") return;
  clip.previewStatus = "processing";
  void store.putClip(clip);
  const previewPath = path.join(config.previewDir, `${clip.id}.mp4`);
  previewQueue = previewQueue.then(() => makePreview(clip.sourcePath, previewPath))
    .then(async () => {
      clip.previewPath = previewPath;
      clip.previewStatus = "ready";
      clip.previewError = undefined;
      await store.putClip(clip);
    })
    .catch(async (error) => {
      console.error(`Preview generation failed for ${clip.name}:`, error);
      clip.previewStatus = "failed";
      clip.previewError = error instanceof Error ? error.message : String(error);
      await store.putClip(clip);
    });
}

async function registerFile(sourcePath: string, displayName?: string) {
  const file = await stat(sourcePath);
  const fingerprint = `${sourcePath}:${file.size}:${file.mtimeMs}`;
  const existing = store.clipByFingerprint(fingerprint);
  if (existing) return existing;
  const clip = await probeClip(sourcePath, displayName);
  await store.putClip(clip);
  queuePreview(clip);
  return clip;
}

for (const clip of store.clips()) {
  if (clip.previewStatus === "processing" || clip.previewStatus === "pending") {
    clip.previewStatus = "pending";
    queuePreview(clip);
  }
}

app.get("/api/config", (_request, response) => {
  response.json({
    sourceDirectory: config.sourceDir,
    audioTrackLabels: config.audioTrackLabels,
    r2Configured: r2Configured(),
    publicBaseUrl: config.r2.publicBaseUrl || null,
    export: { width: 1920, height: 1080, fps: 120, codec: config.videoEncoder, crf: config.crf },
  });
});

app.get("/api/clips", (_request, response) => {
  response.json(store.clips().map(publicClip));
});

app.post("/api/clips/scan", async (_request, response, next) => {
  try {
    const entries = await readdir(config.sourceDir, { withFileTypes: true });
    const discovered = entries
      .filter((entry) => entry.isFile() && mediaExtensions.has(path.extname(entry.name).toLowerCase()))
      .map((entry) => path.join(config.sourceDir, entry.name));
    const imported: Clip[] = [];
    for (const sourcePath of discovered) imported.push(await registerFile(sourcePath));
    response.json({ count: imported.length, clips: store.clips().map(publicClip) });
  } catch (error) {
    next(error);
  }
});

app.post("/api/clips/import", upload.array("clips"), async (request, response, next) => {
  try {
    const files = request.files as Express.Multer.File[];
    if (!files?.length) return response.status(400).json({ error: "Choose at least one video file." });
    const imported: Clip[] = [];
    for (const file of files) {
      const extension = path.extname(file.originalname).toLowerCase();
      if (!mediaExtensions.has(extension)) throw new Error(`Unsupported video type: ${extension || "unknown"}`);
      const unique = createHash("sha256").update(`${file.filename}:${file.originalname}:${randomUUID()}`).digest("hex").slice(0, 16);
      const destination = path.join(config.uploadDir, `${unique}${extension}`);
      await rename(file.path, destination);
      imported.push(await registerFile(destination, file.originalname));
    }
    response.status(201).json({ clips: imported.map(publicClip) });
  } catch (error) {
    next(error);
  }
});

app.get("/api/clips/:id/media", (request, response) => {
  const clip = store.clip(request.params.id);
  if (!clip) return response.status(404).json({ error: "Clip not found." });
  if (clip.previewStatus !== "ready" || !clip.previewPath) {
    return response.status(409).json({ error: "The browser preview is still being prepared." });
  }
  response.sendFile(clip.previewPath);
});

app.delete("/api/clips/:id", async (request, response, next) => {
  try {
    const clip = store.clip(request.params.id);
    if (!clip) return response.status(404).json({ error: "Clip not found." });
    const jobs = store.jobsForClip(clip.id);
    if (jobs.some((job) => ["queued", "transcoding", "uploading"].includes(job.status))) {
      return response.status(409).json({ error: "Wait for the active export to finish before deleting this clip." });
    }
    const removedFileCount = await deleteManagedClipFiles(clip, jobs);
    await store.deleteClip(clip.id);
    response.json({ deleted: true, removedFileCount });
  } catch (error) {
    next(error);
  }
});

app.get("/api/jobs", (_request, response) => response.json(store.jobs()));
app.get("/api/jobs/:id", (request, response) => {
  const job = store.job(request.params.id);
  if (!job) return response.status(404).json({ error: "Job not found." });
  response.json(job);
});

app.post("/api/clips/:id/publish", async (request, response, next) => {
  try {
    if (!r2Configured()) {
      return response.status(503).json({ error: "R2 is not configured. Fill in the R2 values in .env and restart Clip Engine." });
    }
    const clip = store.clip(request.params.id);
    if (!clip) return response.status(404).json({ error: "Clip not found." });
    const start = Number(request.body.start);
    const end = Number(request.body.end);
    const indexes = Array.isArray(request.body.audioStreamIndexes)
      ? request.body.audioStreamIndexes.map(Number)
      : [];
    const validIndexes = new Set(clip.audioTracks.map((track) => track.streamIndex));
    if (!Number.isFinite(start) || start < 0 || !Number.isFinite(end) || end <= start || end > clip.duration + 0.05) {
      return response.status(400).json({ error: "The trim range is invalid." });
    }
    if (indexes.some((index: number) => !validIndexes.has(index))) {
      return response.status(400).json({ error: "One or more selected audio tracks are invalid." });
    }
    const job = await createPublishJob(store, clip, { start, end, audioStreamIndexes: [...new Set<number>(indexes)] });
    response.status(202).json(job);
  } catch (error) {
    next(error);
  }
});

const staticDirectory = path.join(config.root, "dist");
app.use(express.static(staticDirectory));
app.get("/*splat", (_request, response) => response.sendFile(path.join(staticDirectory, "index.html")));

app.use((error: unknown, _request: express.Request, response: express.Response, _next: express.NextFunction) => {
  console.error(error);
  const message = error instanceof Error ? error.message : "An unexpected error occurred.";
  response.status(500).json({ error: message });
});

app.listen(config.port, "127.0.0.1", () => {
  console.log(`Clip Engine is running at http://127.0.0.1:${config.port}`);
  console.log(`Watching for recordings in ${config.sourceDir}`);
});
