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
  previewError?: string;
};

export type Job = {
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

export type AppConfig = {
  sourceDirectory: string;
  audioTrackLabels: string[];
  r2Configured: boolean;
  publicBaseUrl: string | null;
  export: { width: number; height: number; fps: number; codec: string; crf: number };
};
