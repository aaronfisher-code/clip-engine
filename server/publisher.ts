import { randomUUID } from "node:crypto";
import path from "node:path";
import { config } from "./config.js";
import { exportClip, type ExportSelection } from "./media.js";
import { uploadToR2 } from "./r2.js";
import type { Store } from "./store.js";
import type { Clip, PublishJob } from "./types.js";

function safeBaseName(name: string) {
  return path.parse(name).name
    .normalize("NFKD")
    .replace(/[^a-zA-Z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .toLowerCase()
    .slice(0, 60) || "clip";
}

export async function createPublishJob(store: Store, clip: Clip, selection: ExportSelection) {
  const id = randomUUID();
  const suffix = id.slice(0, 8);
  const outputName = `${safeBaseName(clip.name)}-${suffix}.mp4`;
  const job: PublishJob = {
    id,
    clipId: clip.id,
    status: "queued",
    progress: 0,
    createdAt: new Date().toISOString(),
    outputName,
  };
  await store.putJob(job);

  void runJob(store, clip, job, selection);
  return job;
}

async function runJob(store: Store, clip: Clip, job: PublishJob, selection: ExportSelection) {
  const outputPath = path.join(config.exportDir, job.outputName);
  try {
    job.status = "transcoding";
    await store.putJob(job);
    await exportClip(clip.sourcePath, outputPath, selection, (progress) => {
      job.progress = progress * 0.85;
      void store.putJob(job);
    });

    job.status = "uploading";
    job.progress = 0.85;
    await store.putJob(job);
    const date = new Date().toISOString().slice(0, 10);
    job.url = await uploadToR2(outputPath, `${date}/${job.outputName}`, (progress) => {
      job.progress = 0.85 + progress * 0.15;
      void store.putJob(job);
    });
    job.status = "complete";
    job.progress = 1;
    await store.putJob(job);
  } catch (error) {
    job.status = "failed";
    job.error = error instanceof Error ? error.message : String(error);
    await store.putJob(job);
  }
}
