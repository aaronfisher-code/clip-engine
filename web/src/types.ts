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

export type Job = {
  id: string;
  clipId: string;
  status: "queued" | "transcoding" | "uploading" | "complete" | "failed";
  progress: number;
  createdAt: string;
  outputName: string;
  selection?: {
    start: number;
    end: number;
    audioStreamIndexes: number[];
  };
  publishedAt?: string;
  expiresAt?: string;
  url?: string;
  mediaUrl?: string;
  thumbnailUrl?: string;
  remoteClipId?: string;
  error?: string;
};

export type AppConfig = {
  sourceDirectory: string;
  audioTrackLabels: string[];
  r2Configured: boolean;
  authenticated: boolean;
  pendingAccessRequest: boolean;
  publicBaseUrl: string | null;
  apiBaseUrl: string;
  mediaBaseUrl?: string;
  platform: "linux" | "windows" | "macos" | string;
  export: { width: number; height: number; fps: number; codec: string; crf: number };
};

export type CloudUser = {
  id: string;
  username: string;
  displayName: string;
  role: "owner" | "member";
  status: "active" | "revoked";
};

export type AdminUser = CloudUser & {
  deviceCount: number;
  lastSeenAt?: string;
  activeClipCount: number;
  activeBytes: number;
  uploadedBytes: number;
};

export type AccessRequest = {
  id: string;
  username: string;
  displayName: string;
  status: "pending" | "approved" | "denied";
  createdAt: string;
  reviewedAt?: string;
};


export type DeviceSession = { token: string; user: CloudUser; deviceId: string };

export type CloudClip = {
  id: string;
  ownerId: string;
  ownerName: string;
  slug: string;
  title: string;
  status: "uploading" | "published" | "expired" | "deleted" | "failed";
  publishedAt?: string;
  expiresAt?: string;
  duration: number;
  width: number;
  height: number;
  fps: number;
  size: number;
  url?: string;
  mediaUrl?: string;
  thumbnailUrl?: string;
};
