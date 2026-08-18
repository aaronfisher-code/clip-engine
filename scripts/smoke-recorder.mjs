#!/usr/bin/env node

import { existsSync, rmSync, statSync } from "node:fs";
import { basename, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const full = process.argv.includes("--full");
const executable = process.env.CLIP_ENGINE_RECORDER ??
  resolve(root, "target", "debug", process.platform === "win32"
    ? "clip-engine-recorder.exe"
    : "clip-engine-recorder");

if (!existsSync(executable)) {
  console.error(`Recorder helper not found at ${executable}`);
  console.error("Build it first with: cargo build -p clip-engine-recorder --features obs");
  process.exit(2);
}

const result = spawnSync(executable, [full ? "--smoke" : "--probe"], {
  cwd: root,
  encoding: "utf8",
  env: process.env,
});
if (result.error) throw result.error;
if (result.stdout) process.stdout.write(result.stdout);
if (result.stderr) process.stderr.write(result.stderr);
if (result.status !== 0) process.exit(result.status ?? 1);

const payload = parseJsonOutput(result.stdout);
if (full) {
  verifyReplay(payload);
  console.log(
    `Recorder capture smoke passed: ${payload.replayBytes} bytes, ` +
      `${payload.audioRouteCount} configured audio route(s).`,
  );
  cleanupSmokeOutput(payload);
  process.exit(0);
}

verifyCapabilities(payload);
console.log(
  `Recorder runtime smoke passed: ${payload.backend} backend, ` +
    `${payload.screens.length} screen(s), ${payload.audioSources.length} audio source(s).`,
);

function parseJsonOutput(output) {
  try {
    return JSON.parse(output.trim());
  } catch (error) {
    throw new Error(`Recorder did not emit valid JSON: ${error.message}`);
  }
}

function verifyCapabilities(capabilities) {
  const requiredArrays = [
    "screens",
    "audioSources",
    "videoEncoders",
    "audioEncoders",
    "frameRates",
    "diagnostics",
  ];
  for (const key of requiredArrays) {
    if (!Array.isArray(capabilities[key])) {
      throw new Error(`Recorder capabilities are missing the ${key} array`);
    }
  }
  if (typeof capabilities.backend !== "string") {
    throw new Error("Recorder capabilities are missing a capture backend");
  }
  if (capabilities.frameRates.some((range) =>
    !range.min || !range.max ||
    range.min.denominator === 0 ||
    range.max.denominator === 0 ||
    range.min.numerator / range.min.denominator >
      range.max.numerator / range.max.denominator
  )) {
    throw new Error("Recorder reported an invalid frame-rate range");
  }
}

function verifyReplay(smoke) {
  if (typeof smoke.replayPath !== "string" || !existsSync(smoke.replayPath)) {
    throw new Error(`Recorder smoke replay is missing: ${smoke.replayPath}`);
  }
  const bytes = statSync(smoke.replayPath).size;
  if (bytes === 0 || smoke.replayBytes !== bytes) {
    throw new Error(`Recorder smoke replay has invalid size: ${bytes}`);
  }
  verifyEffectiveSettings(smoke);

  const ffprobe = process.env.FFPROBE ?? "ffprobe";
  const probe = spawnSync(ffprobe, [
    "-v", "error",
    "-show_entries",
    "format=duration:stream=index,codec_type,codec_name,width,height,r_frame_rate,bit_rate",
    "-of", "json",
    smoke.replayPath,
  ], { encoding: "utf8" });
  if (probe.error || probe.status !== 0) {
    console.warn("ffprobe unavailable; skipping media metadata verification.");
    return;
  }
  let metadata;
  try {
    metadata = JSON.parse(probe.stdout);
  } catch (error) {
    throw new Error(`ffprobe did not emit JSON: ${error.message}`);
  }
  const streams = Array.isArray(metadata.streams) ? metadata.streams : [];
  const video = streams.find((stream) => stream.codec_type === "video");
  if (!video) {
    throw new Error("Replay does not contain a video stream.");
  }
  const effective = smoke.effectiveSettings;
  if (
    effective.videoCodec &&
    video.codec_name &&
    !video.codec_name.toLowerCase().includes(effective.videoCodec.toLowerCase())
  ) {
    throw new Error(
      `Replay codec ${video.codec_name} does not match effective codec ${effective.videoCodec}.`,
    );
  }
  if (video.width !== effective.outputWidth || video.height !== effective.outputHeight) {
    throw new Error(
      `Replay dimensions ${video.width}x${video.height} do not match ` +
        `${effective.outputWidth}x${effective.outputHeight}.`,
    );
  }
  const frameRate = parseRational(video.r_frame_rate);
  const effectiveFps = rationalValue(effective.fps);
  if (frameRate && effectiveFps && Math.abs(frameRate - effectiveFps) > 0.75) {
    throw new Error(
      `Replay frame rate ${frameRate} does not match effective rate ${effectiveFps}.`,
    );
  }
  const duration = Number(metadata.format?.duration);
  if (!Number.isFinite(duration) || duration <= 0) {
    throw new Error("Replay does not report a positive duration.");
  }
  const streamCount = streams.filter((stream) => stream.codec_type === "audio").length;
  if (streamCount < smoke.audioRouteCount) {
    throw new Error(
      `Replay contains ${streamCount} audio stream(s), expected at least ${smoke.audioRouteCount}.`,
    );
  }
  console.log(`Multi-track replay verification passed: ${streamCount} audio stream(s).`);
  console.log(
    `Replay metadata verified: ${video.codec_name} ${video.width}x${video.height} ` +
      `${frameRate ?? "unknown"} fps, ${duration.toFixed(2)} seconds, ` +
      `${effective.rateControl} rate control.`,
  );
}

function verifyEffectiveSettings(smoke) {
  const effective = smoke.effectiveSettings;
  if (!effective || typeof effective !== "object") {
    throw new Error("Recorder smoke did not return effective encoder settings.");
  }
  for (const key of ["videoEncoder", "rateControl", "containerFormat"]) {
    if (typeof effective[key] !== "string" || effective[key].trim() === "") {
      throw new Error(`Effective recorder settings are missing ${key}.`);
    }
  }
  for (const key of ["outputWidth", "outputHeight"]) {
    if (!(Number(effective[key]) > 0)) {
      throw new Error(`Effective recorder settings contain an invalid ${key}.`);
    }
  }
  if (!(rationalValue(effective.fps) > 0)) {
    throw new Error("Effective recorder settings contain an invalid fps.");
  }
  if (!["mkv", "mp4"].includes(effective.containerFormat.toLowerCase())) {
    throw new Error(`Unsupported effective replay container: ${effective.containerFormat}`);
  }
  const extension = smoke.replayPath.split(".").pop()?.toLowerCase();
  if (extension !== effective.containerFormat.toLowerCase()) {
    throw new Error(
      `Replay container .${extension} does not match effective ${effective.containerFormat}.`,
    );
  }
  if (effective.videoBitrateKbps != null && !(Number(effective.videoBitrateKbps) > 0)) {
    throw new Error("Effective video bitrate is invalid.");
  }
}

function parseRational(value) {
  if (typeof value !== "string") return null;
  const [numerator, denominator] = value.split("/").map(Number);
  if (!(numerator > 0) || !(denominator > 0)) return null;
  return numerator / denominator;
}

function rationalValue(value) {
  if (!value || !(Number(value.numerator) > 0) || !(Number(value.denominator) > 0)) {
    return null;
  }
  return Number(value.numerator) / Number(value.denominator);
}

function cleanupSmokeOutput(smoke) {
  if (
    typeof smoke.outputDirectory !== "string" ||
    !basename(smoke.outputDirectory).startsWith("clip-engine-recorder-smoke-")
  ) {
    return;
  }
  rmSync(smoke.outputDirectory, { recursive: true, force: true });
}
