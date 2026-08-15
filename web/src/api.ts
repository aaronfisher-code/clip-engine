import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import type { AccessRequest, AdminUser, AppConfig, Clip, CloudClip, CloudUser, DeviceSession, Job } from "./types";

const encoder = new TextEncoder();

async function credentialSecret(username: string, password: string) {
  const normalized = username.trim().toLowerCase();
  const key = await crypto.subtle.importKey("raw", encoder.encode(password), "PBKDF2", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits({
    name: "PBKDF2",
    hash: "SHA-256",
    salt: encoder.encode(`clip-engine-password-v1:${normalized}`),
    iterations: 600_000,
  }, key, 256);
  return btoa(String.fromCharCode(...new Uint8Array(bits))).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function inviteToken(invite: string) {
  return invite.trim().split("/").filter(Boolean).at(-1) || invite.trim();
}

export const api = {
  config: () => invoke<AppConfig>("get_config"),
  clips: () => invoke<Clip[]>("list_clips"),
  jobs: () => invoke<Job[]>("list_jobs"),
  scan: async () => {
    const clips = await invoke<Clip[]>("scan_clips");
    return { count: clips.length, clips };
  },
  chooseRecordings: async () => {
    const paths = await open({
      multiple: true,
      directory: false,
      filters: [{ name: "Video recordings", extensions: ["mkv", "mp4", "mov", "webm", "avi", "m4v"] }],
    });
    if (!paths) return { clips: [] as Clip[] };
    return { clips: await invoke<Clip[]>("import_clips", { paths: Array.isArray(paths) ? paths : [paths] }) };
  },
  remove: async (clipId: string) => ({ deleted: true, removedFileCount: await invoke<number>("delete_clip", { id: clipId }) }),
  removeJob: async (jobId: string) => ({ deleted: true, removedFileCount: await invoke<number>("delete_job", { id: jobId }), removedRemoteObjectCount: 0 }),
  publish: (clipId: string, start: number, end: number, audioStreamIndexes: number[]) =>
    invoke<Job>("publish_clip", { clipId, selection: { start, end, audioStreamIndexes } }),
  preparePreview: (clipId: string, force = false) => invoke<Clip>("prepare_preview", { id: clipId, force }),
  previewUrl: (clip: Clip, config?: AppConfig) => {
    if (config?.mediaBaseUrl) return `${config.mediaBaseUrl}/${clip.id}.mp4`;
    return clip.previewPath ? convertFileSrc(clip.previewPath) : undefined;
  },
  sourceUrl: (clip: Clip, config?: AppConfig) => {
    if (config?.mediaBaseUrl) return `${config.mediaBaseUrl}/source/${clip.id}`;
    return convertFileSrc(clip.sourcePath);
  },
  assistedStreamUrl: (clip: Clip, config: AppConfig, start: number, audioStreamIndexes: number[], nonce: number) => {
    if (!config.mediaBaseUrl) return undefined;
    const query = new URLSearchParams({
      start: start.toFixed(6),
      audio: audioStreamIndexes.join(","),
      nonce: String(nonce),
    });
    return `${config.mediaBaseUrl}/stream/${clip.id}?${query}`;
  },
  audioMixUrl: (clip: Clip, config: AppConfig, start: number, audioStreamIndexes: number[], nonce: number) => {
    if (!config.mediaBaseUrl || !audioStreamIndexes.length) return undefined;
    const query = new URLSearchParams({
      start: start.toFixed(6),
      audio: audioStreamIndexes.join(","),
      nonce: String(nonce),
    });
    return `${config.mediaBaseUrl}/audio/${clip.id}?${query}`;
  },
  thumbnailUrl: (clip: Clip, config?: AppConfig) => {
    if (config?.mediaBaseUrl) return `${config.mediaBaseUrl}/${clip.id}.jpg`;
    return clip.previewPath ? convertFileSrc(clip.previewPath.replace(/\.mp4$/i, ".jpg")) : undefined;
  },
  copyText: (value: string) => writeText(value),
  openExternal: (url: string) => openUrl(url),
  redeemInvite: async (invite: string, username: string, password: string, displayName: string, deviceName: string) => {
    const credentialSecretValue = await credentialSecret(username, password);
    return invoke<DeviceSession>("redeem_invite", { inviteToken: inviteToken(invite), username, credentialSecret: credentialSecretValue, displayName, deviceName });
  },
  login: async (username: string, password: string, deviceName: string) =>
    invoke<DeviceSession>("login", { username, credentialSecret: await credentialSecret(username, password), ownerToken: username.trim().toLowerCase() === "admin" ? password : null, deviceName }),
  requestAccess: async (username: string, displayName: string, password: string) =>
    invoke<AccessRequest>("request_access", { username, displayName, credentialSecret: await credentialSecret(username, password) }),
  validatePasswordReset: (token: string, username: string) =>
    invoke<void>("validate_password_reset", { inviteToken: inviteToken(token), username }),
  accessRequestStatus: () => invoke<AccessRequest>("access_request_status"),
  clearAccessRequest: () => invoke<void>("clear_access_request"),
  logout: () => invoke<void>("logout"),
  me: () => invoke<CloudUser>("current_user"),
  cloudClips: () => invoke<CloudClip[]>("cloud_clips"),
  extendCloudClip: (id: string) => invoke<string>("extend_cloud_clip", { id }),
  createPasswordReset: (id: string) => invoke<{ token: string; url: string; username: string; purpose: "password_reset"; expiresAt: string }>("create_password_reset", { id }),
  adminUsers: () => invoke<AdminUser[]>("admin_users"),
  adminAccessRequests: () => invoke<AccessRequest[]>("admin_access_requests"),
  reviewAccessRequest: (id: string, decision: "approved" | "denied") => invoke<void>("review_access_request", { id, decision }),
  setUserStatus: (id: string, status: "active" | "revoked") => invoke<void>("set_user_status", { id, status }),
  checkForUpdate: async () => {
    const update = await check();
    return update ? { version: update.version } : undefined;
  },
  installUpdate: async () => {
    const update = await check();
    if (!update) return false;
    await update.downloadAndInstall();
    await relaunch();
    return true;
  },
};
