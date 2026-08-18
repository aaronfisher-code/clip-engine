#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createWriteStream } from "node:fs";
import { chmod, copyFile, cp, mkdir, readdir, rename, rm, stat } from "node:fs/promises";
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
if (!(await isRuntimeRoot(source))) {
  throw new Error(
    "OBS runtime must contain either data/ plus obs-plugins/, or the standard share/obs/ plus lib/obs-plugins/ layout",
  );
}
await ensureEncoderPlugins(source);
await ensureMuxer(source);
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

async function isRuntimeRoot(root) {
  return (
    ((await isDirectory(join(root, "data"))) &&
      (await isDirectory(join(root, "obs-plugins")))) ||
    ((await isDirectory(join(root, "share", "obs"))) &&
      (await isDirectory(join(root, "lib", "obs-plugins"))))
  );
}

async function ensureEncoderPlugins(root) {
  const missing = [];
  for (const requirement of requiredEncoderPlugins()) {
    if (await findFile(root, requirement.fileName)) continue;

    if (process.platform === "linux" && requirement.allowHostFallback) {
      const systemPlugin = await firstExistingFile(hostPluginCandidates(requirement.module));
      if (systemPlugin) {
        const pluginDirectory = await runtimePluginDirectory(root);
        const destination = join(pluginDirectory, requirement.fileName);
        await copyFile(systemPlugin, destination);
        await chmod(destination, 0o755);
        await copyPluginLocale(root, systemPlugin, requirement.module);
        console.log(`Added the host OBS ${requirement.module} plugin to ${destination}`);
      }
    }

    if (!(await findFile(root, requirement.fileName))) {
      missing.push(requirement.fileName);
    }
  }

  if (missing.length > 0) {
    throw new Error(
      `OBS runtime is missing required encoder plugins: ${missing.join(", ")}. ` +
        "The runtime must include FFmpeg, NVENC, and Intel QSV support; " +
        "AMD AMF on Windows and AMD/Intel VAAPI on Linux are provided by obs-ffmpeg.",
    );
  }
}

async function ensureMuxer(root) {
  const fileName = process.platform === "win32" ? "obs-ffmpeg-mux.exe" : "obs-ffmpeg-mux";
  if (await findFile(root, fileName)) return;
  throw new Error(
    `OBS runtime is missing ${fileName}; replay-buffer saves require the OBS FFmpeg mux helper`,
  );
}

function requiredEncoderPlugins() {
  if (process.platform === "linux") {
    return [
      { fileName: "obs-ffmpeg.so", module: "obs-ffmpeg", allowHostFallback: false },
      { fileName: "obs-nvenc.so", module: "obs-nvenc", allowHostFallback: true },
      { fileName: "obs-qsv11.so", module: "obs-qsv11", allowHostFallback: true },
    ];
  }
  if (process.platform === "win32") {
    return [
      { fileName: "obs-ffmpeg.dll", module: "obs-ffmpeg", allowHostFallback: false },
      { fileName: "obs-nvenc.dll", module: "obs-nvenc", allowHostFallback: false },
      { fileName: "obs-qsv11.dll", module: "obs-qsv11", allowHostFallback: false },
    ];
  }
  return [];
}

function hostPluginCandidates(module) {
  return [
    `/usr/lib/obs-plugins/${module}.so`,
    `/usr/lib64/obs-plugins/${module}.so`,
    `/usr/lib/x86_64-linux-gnu/obs-plugins/${module}.so`,
    `/usr/lib/aarch64-linux-gnu/obs-plugins/${module}.so`,
  ];
}

async function runtimePluginDirectory(root) {
  const candidates =
    process.platform === "win32"
      ? [join(root, "obs-plugins", "64bit"), join(root, "bin", "64bit")]
      : [join(root, "obs-plugins"), join(root, "lib", "obs-plugins")];
  for (const candidate of candidates) {
    if (await isDirectory(candidate)) return candidate;
  }
  throw new Error("OBS runtime has no encoder plugin directory");
}

async function copyPluginLocale(root, systemPlugin, module) {
  const systemData = join(
    dirname(dirname(dirname(systemPlugin))),
    "share",
    "obs",
    "obs-plugins",
    module,
  );
  if (!(await isDirectory(systemData))) return;

  const destinationData = (await isDirectory(join(root, "share", "obs")))
    ? join(root, "share", "obs", "obs-plugins", module)
    : join(root, "data", "obs-plugins", module);
  await rm(destinationData, { recursive: true, force: true });
  await mkdir(dirname(destinationData), { recursive: true });
  await cp(systemData, destinationData, { recursive: true });
}

async function findFile(root, fileName) {
  if (!(await isDirectory(root))) return null;
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.name === fileName && (entry.isFile() || (await isFile(path)))) return path;
    if (entry.isDirectory() || entry.isSymbolicLink()) {
      const match = await findFile(path, fileName);
      if (match) return match;
    }
  }
  return null;
}

async function firstExistingFile(paths) {
  for (const path of paths) {
    if (await isFile(path)) return path;
  }
  return null;
}

async function isDirectory(path) {
  try {
    return (await stat(path)).isDirectory();
  } catch {
    return false;
  }
}

async function isFile(path) {
  try {
    return (await stat(path)).isFile();
  } catch {
    return false;
  }
}
