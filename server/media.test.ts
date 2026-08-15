import assert from "node:assert/strict";
import test from "node:test";
import { buildExportArgs, buildThumbnailArgs, videoEncoderArgs } from "./media.js";

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

test("uses a stream-friendly constrained VBR profile for NVIDIA encoders", () => {
  assert.deepEqual(videoEncoderArgs("h264_nvenc", "medium", 20), [
    "-c:v", "h264_nvenc",
    "-preset", "p5",
    "-tune", "hq",
    "-rc", "vbr",
    "-cq", "20",
    "-b:v", "20M",
    "-maxrate", "30M",
    "-bufsize", "60M",
    "-multipass", "fullres",
    "-spatial-aq", "1",
    "-temporal-aq", "1",
    "-aq-strength", "8",
    "-rc-lookahead", "32",
    "-bf", "3",
    "-b_ref_mode", "middle",
    "-g", "240",
    "-profile:v", "high",
  ]);
});

test("uses a slow, constrained quality profile for libx264", () => {
  assert.deepEqual(videoEncoderArgs("libx264", "slow", 18), [
    "-c:v", "libx264",
    "-preset", "slow",
    "-crf", "18",
    "-maxrate", "30M",
    "-bufsize", "60M",
    "-profile:v", "high",
    "-level:v", "5.1",
    "-g", "240",
    "-keyint_min", "120",
  ]);
});

test("captures a padded 1280x720 thumbnail from one quarter into the export", () => {
  const args = buildThumbnailArgs("clip.mp4", "clip.jpg", 20);
  assert.deepEqual(args.slice(0, 8), [
    "-y", "-hide_banner", "-loglevel", "error", "-ss", "5.000", "-i", "clip.mp4",
  ]);
  assert.ok(args.includes("scale=1280:720:force_original_aspect_ratio=decrease:flags=lanczos,pad=1280:720:(ow-iw)/2:(oh-ih)/2"));
  assert.equal(args.at(-1), "clip.jpg");
});

test("keeps the thumbnail seek inside very short exports", () => {
  const args = buildThumbnailArgs("clip.mp4", "clip.jpg", 0.2);
  assert.equal(args[args.indexOf("-ss") + 1], "0.050");
});
