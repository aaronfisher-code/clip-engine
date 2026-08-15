import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { delimiter, join } from "node:path";

const windows = process.platform === "win32";
const extension = windows ? ".exe" : "";
const destination = join(process.cwd(), "resources", "binaries");
mkdirSync(destination, { recursive: true });

function findExecutable(name) {
  const explicit = process.env[`CLIP_ENGINE_${name.toUpperCase()}`];
  const candidates = explicit
    ? [explicit]
    : (process.env.PATH || "").split(delimiter).map((entry) => join(entry, `${name}${extension}`));
  return candidates.find((candidate) => candidate && existsSync(candidate));
}

for (const name of ["ffmpeg", "ffprobe"]) {
  const target = join(destination, `${name}${extension}`);
  if (existsSync(target)) continue;
  const source = findExecutable(name);
  if (!source) throw new Error(`${name} was not found. Install FFmpeg or set CLIP_ENGINE_${name.toUpperCase()}.`);
  copyFileSync(source, target);
  console.log(`Bundled ${source}`);
}
