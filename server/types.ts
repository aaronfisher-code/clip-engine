export type AudioTrack = {
  streamIndex: number;
  ordinal: number;
  codec: string;
  channels: number;
  channelLayout?: string;
  title?: string;
  language?: string;
};

export type Clip = {
  id: string;
  name: string;
  sourcePath: string;
  fingerprint: string;
  createdAt: string;
  importedAt: string;
  size: number;
  duration: number;
  width: number;
  height: number;
  fps: number;
  videoCodec: string;
  audioTracks: AudioTrack[];
  previewStatus: "pending" | "processing" | "ready" | "failed";
  previewPath?: string;
  previewError?: string;
};

export type PublishJob = {
  id: string;
  clipId: string;
  status: "queued" | "transcoding" | "uploading" | "complete" | "failed";
  progress: number;
  createdAt: string;
  outputName: string;
  url?: string;
  mediaUrl?: string;
  thumbnailUrl?: string;
  error?: string;
};

export type Database = {
  clips: Clip[];
  jobs: PublishJob[];
};
