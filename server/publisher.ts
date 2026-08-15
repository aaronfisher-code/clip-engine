import { randomUUID } from "node:crypto";
import { unlink } from "node:fs/promises";
import path from "node:path";
import { config } from "./config.js";
import { exportClip, makeThumbnail, type ExportSelection } from "./media.js";
import { publicR2Url, uploadFileToR2, uploadTextToR2 } from "./r2.js";
import { createSharePage } from "./share-page.js";
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

function displayName(name: string) {
  return path.parse(name).name.replaceAll("_", " ").replace(/\s+/g, " ").trim() || "Untitled clip";
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
    selection: {
      start: selection.start,
      end: selection.end,
      audioStreamIndexes: [...selection.audioStreamIndexes],
    },
  };
  await store.putJob(job);

  void runJob(store, clip, job, selection);
  return job;
}

async function runJob(store: Store, clip: Clip, job: PublishJob, selection: ExportSelection) {
  const outputPath = path.join(config.exportDir, job.outputName);
  const assetStem = path.parse(job.outputName).name;
  const thumbnailPath = path.join(config.exportDir, `.${assetStem}.thumbnail.jpg`);
  try {
    job.status = "transcoding";
    await store.putJob(job);
    await exportClip(clip.sourcePath, outputPath, selection, (progress) => {
      job.progress = progress * 0.8;
      void store.putJob(job);
    });
    await makeThumbnail(outputPath, thumbnailPath, selection.end - selection.start);

    job.status = "uploading";
    job.progress = 0.8;
    await store.putJob(job);
    const date = new Date().toISOString().slice(0, 10);
    const mediaKey = `media/${date}/${job.outputName}`;
    const thumbnailKey = `thumbnails/${date}/${assetStem}.jpg`;
    const pageKey = `clips/${assetStem}`;
    job.remoteKeys = [mediaKey, thumbnailKey, pageKey];
    await store.putJob(job);

    job.mediaUrl = await uploadFileToR2(outputPath, mediaKey, {
      contentType: "video/mp4",
      contentDisposition: `inline; filename="${job.outputName}"`,
    }, (progress) => {
      job.progress = 0.8 + progress * 0.16;
      void store.putJob(job);
    });
    job.thumbnailUrl = await uploadFileToR2(thumbnailPath, thumbnailKey, {
      contentType: "image/jpeg",
      contentDisposition: `inline; filename="${assetStem}.jpg"`,
    });
    job.progress = 0.98;
    await store.putJob(job);

    const pageUrl = publicR2Url(pageKey);
    const publishedAt = new Date().toISOString();
    job.publishedAt = publishedAt;
    const sharePage = createSharePage({
      title: displayName(clip.name),
      siteName: config.shareSiteName,
      pageUrl,
      videoUrl: job.mediaUrl,
      thumbnailUrl: job.thumbnailUrl,
      duration: selection.end - selection.start,
      width: 1920,
      height: 1080,
      fps: 120,
      publishedAt,
    });
    job.url = await uploadTextToR2(sharePage, pageKey, {
      contentType: "text/html; charset=utf-8",
      contentDisposition: "inline",
    });
    job.status = "complete";
    job.progress = 1;
    await store.putJob(job);
  } catch (error) {
    job.status = "failed";
    job.error = error instanceof Error ? error.message : String(error);
    console.error(`Publish job ${job.id} failed:`, error);
    await store.putJob(job);
  } finally {
    await unlink(thumbnailPath).catch((error: NodeJS.ErrnoException) => {
      if (error.code !== "ENOENT") console.warn(`Could not remove temporary thumbnail ${thumbnailPath}:`, error);
    });
  }
}
