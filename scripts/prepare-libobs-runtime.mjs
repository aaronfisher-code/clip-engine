#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createWriteStream } from "node:fs";
import { mkdir, readdir, rename, rm } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { pipeline } from "node:stream/promises";
import { execFileSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const args = parseArgs(process.argv.slice(2));
const url = args.url ?? process.env.OBS_RUNTIME_URL;
const sha256 = (args.sha256 ?? process.env.OBS_RUNTIME_SHA256 ?? "").toLowerCase();
const archive = resolve(root, args.archive ?? process.env.OBS_RUNTIME_ARCHIVE ?? "target/obs-runtime");
const destination = resolve(root, args.destination ?? process.env.OBS_RUNTIME_DESTINATION ?? "resources/obs");

if (!url || !/^[a-f0-9]{64}$/.test(sha256)) {
  console.error(
    "Usage: OBS_RUNTIME_URL=<url> OBS_RUNTIME_SHA256=<sha256> " +
      "node scripts/prepare-libobs-runtime.mjs [--archive PATH] [--destination PATH]",
  );
  process.exit(2);
}

await mkdir(dirname(archive), { recursive: true });
console.log(`Downloading pinned OBS runtime to ${archive}`);
const response = await fetch(url);
if (!response.ok || !response.body) {
  throw new Error(`OBS runtime download failed: HTTP ${response.status}`);
}
await pipeline(response.body, createWriteStream(archive));

const digest = await sha256File(archive);
if (digest !== sha256) {
  await rm(archive, { force: true });
  throw new Error(`OBS runtime checksum mismatch: expected ${sha256}, got ${digest}`);
}

const unpacked = `${archive}.unpacked`;
await rm(unpacked, { recursive: true, force: true });
await mkdir(unpacked, { recursive: true });
extract(archive, unpacked);

const entries = await readdir(unpacked, { withFileTypes: true });
const source =
  entries.length === 1 && entries[0].isDirectory()
    ? join(unpacked, entries[0].name)
    : unpacked;
const runtimeEntries = await readdir(source, { withFileTypes: true });
for (const requiredDirectory of ["data", "obs-plugins"]) {
  if (!runtimeEntries.some((entry) => entry.isDirectory() && entry.name === requiredDirectory)) {
    throw new Error(`OBS runtime is missing required ${requiredDirectory}/ directory`);
  }
}
await rm(destination, { recursive: true, force: true });
await mkdir(dirname(destination), { recursive: true });
await rename(source, destination);
await rm(unpacked, { recursive: true, force: true });
console.log(`Installed the verified OBS runtime at ${destination}`);

function parseArgs(values) {
  const parsed = {};
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (!value.startsWith("--")) throw new Error(`Unknown argument ${value}`);
    const key = value.slice(2).replaceAll("-", "_");
    parsed[key] = values[++index];
  }
  return parsed;
}

async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

function extract(path, output) {
  if (path.endsWith(".zip")) {
    if (process.platform === "win32") {
      execFileSync("tar", ["-xf", path, "-C", output], { stdio: "inherit" });
    } else {
      execFileSync("unzip", ["-q", path, "-d", output], { stdio: "inherit" });
    }
    return;
  }
  execFileSync("tar", ["-xf", path, "-C", output], { stdio: "inherit" });
}
