#!/usr/bin/env node

import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const executable = process.env.CLIP_ENGINE_RECORDER ??
  resolve(root, "target", "debug", process.platform === "win32"
    ? "clip-engine-recorder.exe"
    : "clip-engine-recorder");

if (!existsSync(executable)) {
  console.error(`Recorder helper not found at ${executable}`);
  console.error("Build it first with: cargo build -p clip-engine-recorder --features obs");
  process.exit(2);
}

const result = spawnSync(executable, ["--probe"], {
  cwd: root,
  stdio: "inherit",
  env: process.env,
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
