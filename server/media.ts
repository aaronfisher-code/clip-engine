import { execFile, spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { stat } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";
import { config } from "./config.js";
import type { AudioTrack, Clip } from "./types.js";

const execFileAsync = promisify(execFile);

type ProbeStream = {
  index: number;
  codec_type: "video" | "audio" | string;
  codec_name?: string;
  width?: number;
  height?: number;
  avg_frame_rate?: string;
  r_frame_rate?: string;
  channels?: number;
  channel_layout?: string;
  tags?: { title?: string; language?: string };
};

type ProbeResult = {
  streams: ProbeStream[];
  format: { duration?: string; size?: string; tags?: { creation_time?: string } };
};

function rate(value?: string) {
  if (!value) return 0;
  const [numerator = 0, denominator = 1] = value.split("/").map(Number);
  return denominator ? numerator / denominator : 0;
}

export async function probeClip(sourcePath: string, name = path.basename(sourcePath)): Promise<Clip> {
  const file = await stat(sourcePath);
  const { stdout } = await execFileAsync(config.ffprobe, [
    "-v", "error",
    "-show_format",
    "-show_streams",
    "-of", "json",
    sourcePath,
  ], { maxBuffer: 8 * 1024 * 1024 });
  const probe = JSON.parse(stdout) as ProbeResult;
  const video = probe.streams.find((stream) => stream.codec_type === "video");
  if (!video) throw new Error("No video stream was found in this file.");
  const audioTracks: AudioTrack[] = probe.streams
    .filter((stream) => stream.codec_type === "audio")
    .map((stream, ordinal) => ({
      streamIndex: stream.index,
      ordinal,
      codec: stream.codec_name || "unknown",
      channels: stream.channels || 0,
      channelLayout: stream.channel_layout,
      title: stream.tags?.title,
      language: stream.tags?.language,
    }));
  const creationTime = probe.format.tags?.creation_time;
  const fingerprint = `${sourcePath}:${file.size}:${file.mtimeMs}`;

  return {
    id: randomUUID(),
    name,
    sourcePath,
    fingerprint,
    createdAt: creationTime && !Number.isNaN(Date.parse(creationTime))
      ? new Date(creationTime).toISOString()
      : file.mtime.toISOString(),
    importedAt: new Date().toISOString(),
    size: file.size,
    duration: Number(probe.format.duration || 0),
    width: video.width || 0,
    height: video.height || 0,
    fps: rate(video.avg_frame_rate) || rate(video.r_frame_rate),
    videoCodec: video.codec_name || "unknown",
    audioTracks,
    previewStatus: "pending",
  };
}

function runFfmpeg(args: string[], onStdout?: (text: string) => void) {
  return new Promise<void>((resolve, reject) => {
    const child = spawn(config.ffmpeg, args, { stdio: ["ignore", "pipe", "pipe"] });
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => onStdout?.(chunk));
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      stderr = (stderr + chunk).slice(-16_000);
    });
    child.once("error", reject);
    child.once("close", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`FFmpeg exited with code ${code}. ${stderr.trim()}`));
    });
  });
}

export async function makePreview(sourcePath: string, outputPath: string) {
  await runFfmpeg([
    "-y", "-hide_banner", "-loglevel", "error",
    "-i", sourcePath,
    "-map", "0:v:0", "-map", "0:a:0?",
    "-vf", "scale=1280:-2:force_original_aspect_ratio=decrease,fps=30",
    "-c:v", "libx264", "-preset", "veryfast", "-crf", "28",
    "-pix_fmt", "yuv420p",
    "-c:a", "aac", "-b:a", "128k",
    "-movflags", "+faststart",
    outputPath,
  ]);
}

export type ExportSelection = {
  start: number;
  end: number;
  audioStreamIndexes: number[];
};

export function videoEncoderArgs(encoder: string, preset: string, quality: number) {
  if (encoder.endsWith("_nvenc")) {
    return ["-c:v", encoder, "-preset", preset === "medium" ? "p5" : preset, "-rc", "vbr", "-cq", String(quality), "-b:v", "0"];
  }
  if (encoder.endsWith("_qsv")) {
    return ["-c:v", encoder, "-preset", preset, "-global_quality", String(quality)];
  }
  if (encoder.endsWith("_amf")) {
    return ["-c:v", encoder, "-quality", preset === "medium" ? "balanced" : preset, "-rc", "cqp", "-qp_i", String(quality), "-qp_p", String(quality)];
  }
  return ["-c:v", encoder, "-preset", preset, "-crf", String(quality)];
}

export function buildExportArgs(
  sourcePath: string,
  outputPath: string,
  selection: ExportSelection,
) {
  const duration = selection.end - selection.start;
  const args = [
    "-y", "-hide_banner", "-loglevel", "warning",
    "-ss", selection.start.toFixed(6),
    "-i", sourcePath,
    "-t", duration.toFixed(6),
  ];

  if (selection.audioStreamIndexes.length > 1) {
    const inputs = selection.audioStreamIndexes
      .map((index, position) => `[0:${index}]aresample=async=1:first_pts=0[a${position}]`)
      .join(";");
    const pads = selection.audioStreamIndexes.map((_, position) => `[a${position}]`).join("");
    args.push("-filter_complex", `${inputs};${pads}amix=inputs=${selection.audioStreamIndexes.length}:duration=longest:normalize=1[aout]`);
  }

  args.push(
    "-map", "0:v:0",
    "-vf", "scale=1920:1080:force_original_aspect_ratio=decrease:flags=lanczos,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,fps=120",
    ...videoEncoderArgs(config.videoEncoder, config.preset, config.crf),
    "-pix_fmt", "yuv420p",
  );

  if (selection.audioStreamIndexes.length > 1) args.push("-map", "[aout]", "-c:a", "aac", "-b:a", "192k");
  else if (selection.audioStreamIndexes.length === 1) {
    args.push("-map", `0:${selection.audioStreamIndexes[0]}`, "-c:a", "aac", "-b:a", "192k");
  } else args.push("-an");

  args.push(
    "-movflags", "+faststart",
    "-progress", "pipe:1", "-nostats",
    outputPath,
  );
  return args;
}

export async function exportClip(
  sourcePath: string,
  outputPath: string,
  selection: ExportSelection,
  onProgress: (progress: number) => void,
) {
  const durationUs = (selection.end - selection.start) * 1_000_000;
  let buffer = "";
  await runFfmpeg(buildExportArgs(sourcePath, outputPath, selection), (chunk) => {
    buffer += chunk;
    const lines = buffer.split("\n");
    buffer = lines.pop() || "";
    for (const line of lines) {
      const match = line.match(/^out_time_us=(\d+)/);
      if (match) onProgress(Math.min(1, Number(match[1]) / durationUs));
    }
  });
  onProgress(1);
}
