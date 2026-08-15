export type UserRole = "owner" | "member";
export type UserStatus = "active" | "revoked";
export type ClipStatus = "uploading" | "published" | "expired" | "deleted" | "failed";

export type CloudUser = {
  id: string;
  username: string;
  displayName: string;
  role: UserRole;
  status: UserStatus;
};

export type CloudClip = {
  id: string;
  ownerId: string;
  ownerName: string;
  slug: string;
  title: string;
  status: ClipStatus;
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

export type UploadIntent = {
  title: string;
  videoSize: number;
  thumbnailSize: number;
  duration: number;
  width: number;
  height: number;
  fps: number;
};

export type R2TemporaryCredentials = {
  accessKeyId: string;
  secretAccessKey: string;
  sessionToken: string;
  endpoint: string;
  bucket: string;
  expiresAt: string;
};

export type CreatedUpload = {
  uploadId: string;
  clipId: string;
  videoKey: string;
  thumbnailKey: string;
  credentials: R2TemporaryCredentials;
};

export type CompleteUpload = { videoEtag?: string; thumbnailEtag?: string };

export type Invite = {
  id: string;
  username?: string;
  role: UserRole;
  purpose: "enroll" | "password_reset";
  expiresAt: string;
  redeemedAt?: string;
};

export type CreatedInvite = Invite & { token: string; url: string };

export type AccessRequestStatus = "pending" | "approved" | "denied";

export type AccessRequest = {
  id: string;
  username: string;
  displayName: string;
  status: AccessRequestStatus;
  createdAt: string;
  reviewedAt?: string;
};

export type CreatedAccessRequest = AccessRequest & { requestToken: string };

export type DeviceSession = {
  token: string;
  user: CloudUser;
  deviceId: string;
};

export type ApiError = { error: string; code?: string };
