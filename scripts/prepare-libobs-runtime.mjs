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
await downloadArchive(url, archive);

const digest = await sha256File(archive);
if (digest !== sha256) {
  await rm(archive, { force: true });
  throw new Error(`OBS runtime checksum mismatch: expected ${sha256}, got ${digest}`);
}

const unpacked = `${archive}.unpacked`;
await rm(unpacked, { recursive: true, force: true });
await mkdir(unpacked, { recursive: true });
extract(archive, unpacked);

const source = await findRuntimeRoot(unpacked);
if (!source) {
  throw new Error(
    "OBS runtime must contain either data/ plus obs-plugins/, or the standard share/obs/ plus lib/obs-plugins/ layout",
  );
}
await normalizeRuntimeRoot(source);
if (!(await isRuntimeRoot(source))) {
  throw new Error("OBS runtime could not be normalized to a supported layout");
}
await pruneDesktopExecutable(source);
await pruneRecorderIncompatibleModules(source);
await ensureCapturePlugins(source);
await ensureEncoderPlugins(source);
await ensureEncoderHelpers(source);
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

async function downloadArchive(url, destination) {
  const temporary = `${destination}.part`;
  const attempts = 4;
  const timeoutMs = 120_000;
  await rm(temporary, { force: true });

  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), timeoutMs);
      try {
        const response = await fetch(url, { signal: controller.signal });
        if (!response.ok || !response.body) {
          const error = new Error(`OBS runtime download failed: HTTP ${response.status}`);
          error.retryable =
            !response.body ||
            response.status === 408 ||
            response.status === 425 ||
            response.status === 429 ||
            response.status >= 500;
          throw error;
        }
        await pipeline(response.body, createWriteStream(temporary));
      } finally {
        clearTimeout(timeout);
      }

      await rm(destination, { force: true });
      await rename(temporary, destination);
      return;
    } catch (error) {
      await rm(temporary, { force: true });
      if (attempt === attempts || !isRetryableDownloadError(error)) throw error;

      const delayMs = 1_000 * 2 ** (attempt - 1);
      console.warn(
        `OBS runtime download attempt ${attempt}/${attempts} failed; ` +
          `retrying in ${delayMs}ms: ${errorMessage(error)}`,
      );
      await sleep(delayMs);
    }
  }
}

function isRetryableDownloadError(error) {
  if (error?.retryable === true || error?.name === "AbortError") return true;
  const code = error?.cause?.code ?? error?.code;
  return (
    error?.message === "fetch failed" ||
    ["ECONNRESET", "ECONNREFUSED", "ETIMEDOUT", "EAI_AGAIN", "ENETUNREACH"].includes(code)
  );
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

function extract(path, output) {
  if (path.endsWith(".deb")) {
    execFileSync("dpkg-deb", ["-x", path, output], { stdio: "inherit" });
    return;
  }
  if (path.endsWith(".zip")) {
    if (process.platform === "win32") {
      execFileSync(
        "powershell.exe",
        [
          "-NoProfile",
          "-NonInteractive",
          "-Command",
          "Expand-Archive -LiteralPath $env:CLIP_ENGINE_OBS_ARCHIVE -DestinationPath $env:CLIP_ENGINE_OBS_OUTPUT -Force",
        ],
        {
          stdio: "inherit",
          env: {
            ...process.env,
            CLIP_ENGINE_OBS_ARCHIVE: path,
            CLIP_ENGINE_OBS_OUTPUT: output,
          },
        },
      );
    } else {
      execFileSync("unzip", ["-q", path, "-d", output], { stdio: "inherit" });
    }
    return;
  }
  execFileSync("tar", ["-xf", path, "-C", output], { stdio: "inherit" });
}

async function isRuntimeRoot(root) {
  const hasStandardPlugins = await isDirectory(join(root, "lib", "obs-plugins"));
  const hasMultiArchPlugins = Boolean(await findMultiArchLibrary(root));
  return (
    ((await isDirectory(join(root, "data"))) &&
      (await isDirectory(join(root, "obs-plugins")))) ||
    ((await isDirectory(join(root, "share", "obs"))) &&
      (hasStandardPlugins || hasMultiArchPlugins))
  );
}

async function findRuntimeRoot(root) {
  if (await isRuntimeRoot(root)) return root;
  for (const entry of await readdir(root, { withFileTypes: true })) {
    if (!entry.isDirectory() && !entry.isSymbolicLink()) continue;
    const candidate = join(root, entry.name);
    const source = await findRuntimeRoot(candidate);
    if (source) return source;
  }
  return null;
}

async function findMultiArchLibrary(root) {
  const libRoot = join(root, "lib");
  if (!(await isDirectory(libRoot))) return null;
  for (const entry of await readdir(libRoot, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.endsWith("-linux-gnu")) continue;
    const candidate = join(libRoot, entry.name);
    if (await isDirectory(join(candidate, "obs-plugins"))) return candidate;
  }
  return null;
}

async function normalizeRuntimeRoot(root) {
  const multiArchLibrary = await findMultiArchLibrary(root);
  if (!multiArchLibrary) return;

  const libRoot = join(root, "lib");
  for (const entry of await readdir(multiArchLibrary, { withFileTypes: true })) {
    const source = join(multiArchLibrary, entry.name);
    const destination = join(libRoot, entry.name);
    await rm(destination, { recursive: true, force: true });
    await rename(source, destination);
  }
  await rm(multiArchLibrary, { recursive: true, force: true });
}

async function pruneDesktopExecutable(root) {
  if (process.platform !== "linux") return;
  await rm(join(root, "bin", "obs"), { force: true });
  await rm(join(root, "obs"), { force: true });
  for (const pluginPath of [
    join(root, "lib", "obs-plugins", "obs-websocket.so"),
    join(root, "obs-plugins", "obs-websocket.so"),
  ]) {
    await rm(pluginPath, { force: true });
  }
  for (const dataPath of [
    join(root, "share", "obs", "obs-plugins", "obs-websocket"),
    join(root, "data", "obs-plugins", "obs-websocket"),
  ]) {
    await rm(dataPath, { recursive: true, force: true });
  }
}

async function pruneRecorderIncompatibleModules(root) {
  // The recorder embeds libobs without the OBS desktop frontend. The browser
  // module assumes that frontend API and starts CEF during module loading.
  // Loading it in this standalone process can crash the helper before IPC
  // starts, especially under Wayland.
  // Its shared libraries also live in obs-plugins/, where libobs scans every
  // .so as a possible module.
  const browserFiles = new Set([
    "obs-browser.so",
    "obs-browser.dll",
    "libcef.so",
    "libcef.dll",
    "libEGL.so",
    "libEGL.dll",
    "libGLESv2.so",
    "libGLESv2.dll",
    "libvk_swiftshader.so",
    "libvk_swiftshader.dll",
    "libvulkan.so.1",
    "icudtl.dat",
    "v8_context_snapshot.bin",
    "snapshot_blob.bin",
    "natives_blob.bin",
    "resources.pak",
    "cef.pak",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "vk_swiftshader_icd.json",
    "obs-browser-page",
    "obs-browser-page.exe",
    "chrome-sandbox",
  ]);
  const browserDirectories = [
    join(root, "obs-plugins"),
    join(root, "obs-plugins", "64bit"),
    join(root, "lib", "obs-plugins"),
    join(root, "bin", "64bit"),
  ];
  for (const directory of browserDirectories) {
    await removeNamedEntries(directory, browserFiles);
  }
  for (const dataPath of [
    join(root, "data", "obs-plugins", "obs-browser"),
    join(root, "share", "obs", "obs-plugins", "obs-browser"),
  ]) {
    await rm(dataPath, { recursive: true, force: true });
  }
}

async function removeNamedEntries(directory, names) {
  if (!(await isDirectory(directory))) return;
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (names.has(entry.name)) {
      await rm(path, { recursive: true, force: true });
    } else if (entry.isDirectory() && !entry.isSymbolicLink()) {
      await removeNamedEntries(path, names);
    }
  }
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

async function ensureCapturePlugins(root) {
  const missing = [];
  for (const requirement of requiredCapturePlugins()) {
    if (!(await findFile(root, requirement.fileName))) {
      missing.push(`${requirement.fileName} (${requirement.purpose})`);
    }
  }

  if (missing.length > 0) {
    throw new Error(
      `OBS runtime is missing required capture plugins: ${missing.join(", ")}. ` +
        "The bundled recorder requires the platform display and audio capture modules.",
    );
  }
}

async function ensureEncoderHelpers(root) {
  const missing = [];
  for (const requirement of requiredEncoderHelpers()) {
    if (!(await findFile(root, requirement.fileName))) {
      missing.push(`${requirement.fileName} (${requirement.purpose})`);
    }
  }

  if (missing.length > 0) {
    throw new Error(
      `OBS runtime is missing required encoder helpers: ${missing.join(", ")}. ` +
        "Hardware encoder plugins cannot complete their capability checks without them.",
    );
  }
}

function requiredEncoderHelpers() {
  if (process.platform === "linux") {
    return [{ fileName: "obs-nvenc-test", purpose: "NVIDIA NVENC capability check" }];
  }
  if (process.platform === "win32") {
    return [
      { fileName: "obs-nvenc-test.exe", purpose: "NVIDIA NVENC capability check" },
      { fileName: "obs-qsv-test.exe", purpose: "Intel Quick Sync capability check" },
    ];
  }
  return [];
}

function requiredCapturePlugins() {
  if (process.platform === "linux") {
    return [
      { fileName: "linux-capture.so", purpose: "X11 display capture" },
      { fileName: "linux-pipewire.so", purpose: "Wayland display capture" },
      {
        fileName: "linux-pulseaudio.so",
        purpose: "PulseAudio system and microphone fallback",
      },
    ];
  }
  if (process.platform === "win32") {
    return [
      { fileName: "win-capture.dll", purpose: "Windows display capture" },
      {
        fileName: "libobs-winrt.dll",
        purpose: "Windows Graphics Capture support for multi-adapter displays",
      },
      {
        fileName: "win-wasapi.dll",
        purpose: "WASAPI system, microphone, and application audio",
      },
    ];
  }
  return [];
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
