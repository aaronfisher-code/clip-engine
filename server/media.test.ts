import assert from "node:assert/strict";
import test from "node:test";
import { buildExportArgs, videoEncoderArgs } from "./media.js";

test("builds a silent, bounded 1080p120 export", () => {
  const args = buildExportArgs("source.mkv", "output.mp4", {
    start: 1.25,
    end: 3.75,
    audioStreamIndexes: [],
  });
  assert.deepEqual(args.slice(0, 10), [
    "-y", "-hide_banner", "-loglevel", "warning", "-ss", "1.250000", "-i", "source.mkv", "-t", "2.500000",
  ]);
  assert.ok(args.includes("-an"));
  assert.ok(args.includes("scale=1920:1080:force_original_aspect_ratio=decrease:flags=lanczos,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,fps=120"));
});

test("maps one selected audio stream by its absolute FFmpeg index", () => {
  const args = buildExportArgs("source.mkv", "output.mp4", {
    start: 0,
    end: 10,
    audioStreamIndexes: [3],
  });
  const maps = args.flatMap((argument, index) => argument === "-map" ? [args[index + 1]] : []);
  assert.deepEqual(maps, ["0:v:0", "0:3"]);
  assert.ok(!args.includes("-filter_complex"));
});

test("mixes multiple selected audio streams into one playback track", () => {
  const args = buildExportArgs("source.mkv", "output.mp4", {
    start: 0,
    end: 10,
    audioStreamIndexes: [1, 4],
  });
  const filter = args[args.indexOf("-filter_complex") + 1];
  assert.match(filter, /^\[0:1\].*\[0:4\].*amix=inputs=2.*\[aout\]$/);
  assert.equal(args[args.lastIndexOf("-map") + 1], "[aout]");
});

test("uses constant-quality flags appropriate to NVIDIA encoders", () => {
  assert.deepEqual(videoEncoderArgs("h264_nvenc", "medium", 20), [
    "-c:v", "h264_nvenc", "-preset", "p5", "-rc", "vbr", "-cq", "20", "-b:v", "0",
  ]);
});
