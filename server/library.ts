import { lstat, unlink } from "node:fs/promises";
import path from "node:path";
import { config } from "./config.js";
import type { Clip, PublishJob } from "./types.js";

type ManagedRoots = {
  source: string[];
  preview: string;
  exports: string;
};

function pathIsInside(root: string, candidate: string) {
  const relative = path.relative(path.resolve(root), path.resolve(candidate));
  return relative !== ""
    && relative !== ".."
    && !relative.startsWith(`..${path.sep}`)
    && !path.isAbsolute(relative);
}

export function managedDeletionTargets(
  clip: Clip,
  jobs: PublishJob[],
  roots: ManagedRoots = {
    source: [config.sourceDir, config.uploadDir],
    preview: config.previewDir,
    exports: config.exportDir,
  },
) {
  const targets: string[] = [];
  if (roots.source.some((root) => pathIsInside(root, clip.sourcePath))) {
    targets.push(path.resolve(clip.sourcePath));
  }
  if (clip.previewPath && pathIsInside(roots.preview, clip.previewPath)) {
    targets.push(path.resolve(clip.previewPath));
  }
  for (const job of jobs) {
    const outputPath = path.resolve(roots.exports, job.outputName);
    if (pathIsInside(roots.exports, outputPath)) targets.push(outputPath);
  }
  return [...new Set(targets)];
}

export async function deleteManagedClipFiles(clip: Clip, jobs: PublishJob[]) {
  const targets = managedDeletionTargets(clip, jobs);
  return deleteExistingManagedFiles(targets);
}

export function managedJobDeletionTargets(
  job: PublishJob,
  exportRoot = config.exportDir,
) {
  const outputPath = path.resolve(exportRoot, job.outputName);
  return pathIsInside(exportRoot, outputPath) ? [outputPath] : [];
}

export async function deleteManagedJobFiles(job: PublishJob) {
  return deleteExistingManagedFiles(managedJobDeletionTargets(job));
}

async function deleteExistingManagedFiles(targets: string[]) {
  const existing: string[] = [];

  for (const target of targets) {
    try {
      const file = await lstat(target);
      if (!file.isFile() && !file.isSymbolicLink()) {
        throw new Error(`Refusing to delete a non-file clip path: ${target}`);
      }
      existing.push(target);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }

  await Promise.all(existing.map((target) => unlink(target)));
  return existing.length;
}
