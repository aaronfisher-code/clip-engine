import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { config } from "./config.js";
import type { Clip, Database, PublishJob } from "./types.js";

const emptyDatabase: Database = { clips: [], jobs: [] };

export class Store {
  private data: Database = structuredClone(emptyDatabase);
  private saveQueue: Promise<void> = Promise.resolve();

  async init() {
    await Promise.all([
      mkdir(config.dataDir, { recursive: true }),
      mkdir(config.sourceDir, { recursive: true }),
      mkdir(config.uploadDir, { recursive: true }),
      mkdir(config.previewDir, { recursive: true }),
      mkdir(config.exportDir, { recursive: true }),
    ]);
    try {
      this.data = JSON.parse(await readFile(config.databasePath, "utf8")) as Database;
      this.data.jobs = this.data.jobs.map((job) =>
        ["queued", "transcoding", "uploading"].includes(job.status)
          ? { ...job, status: "failed", error: "The app stopped before this job finished." }
          : job,
      );
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      await this.save();
    }
  }

  clips() {
    return [...this.data.clips].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  }

  clip(id: string) {
    return this.data.clips.find((clip) => clip.id === id);
  }

  clipByFingerprint(fingerprint: string) {
    return this.data.clips.find((clip) => clip.fingerprint === fingerprint);
  }

  async putClip(clip: Clip) {
    const index = this.data.clips.findIndex((item) => item.id === clip.id);
    if (index === -1) this.data.clips.push(clip);
    else this.data.clips[index] = clip;
    await this.save();
  }

  async deleteClip(id: string) {
    this.data.clips = this.data.clips.filter((clip) => clip.id !== id);
    this.data.jobs = this.data.jobs.filter((job) => job.clipId !== id);
    await this.save();
  }

  jobs() {
    return [...this.data.jobs].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  }

  job(id: string) {
    return this.data.jobs.find((job) => job.id === id);
  }

  jobsForClip(clipId: string) {
    return this.data.jobs.filter((job) => job.clipId === clipId);
  }

  async putJob(job: PublishJob) {
    const index = this.data.jobs.findIndex((item) => item.id === job.id);
    if (index === -1) this.data.jobs.push(job);
    else this.data.jobs[index] = job;
    await this.save();
  }

  private save() {
    this.saveQueue = this.saveQueue.then(async () => {
      const temporaryPath = path.join(config.dataDir, `.clip-engine-${process.pid}.tmp`);
      await writeFile(temporaryPath, JSON.stringify(this.data, null, 2));
      await rename(temporaryPath, config.databasePath);
    });
    return this.saveQueue;
  }
}
