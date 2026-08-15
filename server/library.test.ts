import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { managedDeletionTargets } from "./library.js";
import type { Clip, PublishJob } from "./types.js";

const roots = {
  source: ["/managed/inbox", "/managed/uploads"],
  preview: "/managed/previews",
  exports: "/managed/exports",
};

function clip(sourcePath: string, previewPath?: string) {
  return { id: "clip-1", sourcePath, previewPath } as Clip;
}

function job(outputName: string) {
  return { id: "job-1", clipId: "clip-1", outputName } as PublishJob;
}

test("deletes only source, preview, and export files within managed roots", () => {
  assert.deepEqual(managedDeletionTargets(
    clip("/managed/inbox/replay.mkv", "/managed/previews/clip-1.mp4"),
    [job("published.mp4")],
    roots,
  ), [
    path.resolve("/managed/inbox/replay.mkv"),
    path.resolve("/managed/previews/clip-1.mp4"),
    path.resolve("/managed/exports/published.mp4"),
  ]);
});

test("does not delete external files or accept traversal in stored output names", () => {
  assert.deepEqual(managedDeletionTargets(
    clip("/home/user/video.mkv", "/tmp/preview.mp4"),
    [job("../../outside.mp4")],
    roots,
  ), []);
});
