import path from "node:path";
import dotenv from "dotenv";

dotenv.config({ quiet: true });

function positiveNumber(value: string | undefined, fallback: number) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

const root = process.cwd();
const dataDir = path.resolve(root, process.env.CLIP_DATA_DIR || "data");

export const config = {
  root,
  dataDir,
  sourceDir: path.resolve(root, process.env.CLIP_SOURCE_DIR || path.join(dataDir, "inbox")),
  uploadDir: path.join(dataDir, "sources"),
  previewDir: path.join(dataDir, "previews"),
  exportDir: path.join(dataDir, "exports"),
  databasePath: path.join(dataDir, "clip-engine.json"),
  port: positiveNumber(process.env.PORT, 4317),
  ffmpeg: process.env.FFMPEG_PATH || "ffmpeg",
  ffprobe: process.env.FFPROBE_PATH || "ffprobe",
  videoEncoder: process.env.FFMPEG_VIDEO_ENCODER || "libx264",
  preset: process.env.FFMPEG_PRESET || "medium",
  crf: positiveNumber(process.env.FFMPEG_CRF, 20),
  r2: {
    accountId: process.env.R2_ACCOUNT_ID || "",
    accessKeyId: process.env.R2_ACCESS_KEY_ID || "",
    secretAccessKey: process.env.R2_SECRET_ACCESS_KEY || "",
    bucket: process.env.R2_BUCKET || "",
    publicBaseUrl: (process.env.R2_PUBLIC_BASE_URL || "").replace(/\/$/, ""),
  },
};

export function r2Configured() {
  return Object.values(config.r2).every(Boolean);
}
