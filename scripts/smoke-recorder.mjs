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

  const ffprobe = process.env.FFPROBE ?? "ffprobe";
  const probe = spawnSync(ffprobe, [
    "-v", "error",
    "-select_streams", "a",
    "-show_entries", "stream=index",
    "-of", "csv=p=0",
    smoke.replayPath,
  ], { encoding: "utf8" });
  if (probe.error || probe.status !== 0) {
    console.warn("ffprobe unavailable; skipping multi-track replay verification.");
    return;
  }
  const streamCount = probe.stdout.trim() === "" ? 0 : probe.stdout.trim().split(/\r?\n/).length;
  if (streamCount < smoke.audioRouteCount) {
    throw new Error(
      `Replay contains ${streamCount} audio stream(s), expected at least ${smoke.audioRouteCount}.`,
    );
  }
  console.log(`Multi-track replay verification passed: ${streamCount} audio stream(s).`);
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
