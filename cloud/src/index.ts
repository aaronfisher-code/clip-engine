import type {
  CloudClip,
  CompleteUpload,
  CreatedLicence,
  CreatedUpload,
  DeviceSession,
  Licence,
  UploadIntent,
} from "@clip-engine/contracts";
import { authenticate, randomToken, secureEqual, tokenHash } from "./security";
import { body, cors, errorResponse, HttpError, json } from "./http";
import { gonePage, sharePage } from "./share";
import { createR2TemporaryCredentials } from "./r2-credentials";
import type { Env, Principal } from "./types";

const isoNow = () => new Date().toISOString();
const addSeconds = (seconds: number) => new Date(Date.now() + seconds * 1_000).toISOString();
const addDays = (days: number) => addSeconds(days * 86_400);

function cleanText(value: unknown, field: string, maximum: number) {
  if (typeof value !== "string") throw new HttpError(400, `${field} is required.`);
  const clean = value.trim().replace(/\s+/g, " ");
  if (!clean || clean.length > maximum) throw new HttpError(400, `${field} must be between 1 and ${maximum} characters.`);
  return clean;
}

function internalEmail(id: string) {
  return `${id}@no-email.invalid`;
}

function number(value: unknown, field: string, minimum: number, maximum: number) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < minimum || value > maximum) {
    throw new HttpError(400, `${field} is outside the allowed range.`);
  }
  return value;
}

export function validateUploadIntent(value: UploadIntent, maximumBytes: number): UploadIntent {
  return {
    title: cleanText(value.title, "Title", 160),
    videoSize: number(value.videoSize, "Video size", 1, maximumBytes),
    thumbnailSize: number(value.thumbnailSize, "Thumbnail size", 1, 20 * 1024 * 1024),
    duration: number(value.duration, "Duration", 0.05, 60 * 60),
    width: Math.round(number(value.width, "Width", 16, 16384)),
    height: Math.round(number(value.height, "Height", 16, 16384)),
    fps: number(value.fps, "Frame rate", 1, 240),
  };
}

async function principal(request: Request, env: Env) {
  const value = await authenticate(request, env);
  if (!value) throw new HttpError(401, "Authentication is required.", "unauthorized");
  return value;
}

function owner(value: Principal) {
  if (value.role !== "owner") throw new HttpError(403, "Owner access is required.", "forbidden");
}

async function audit(env: Env, actorId: string | null, action: string, targetType: string, targetId?: string, details?: unknown) {
  await env.DB.prepare(`INSERT INTO audit_events (id, actor_id, action, target_type, target_id, details, created_at)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)`)
    .bind(crypto.randomUUID(), actorId, action, targetType, targetId || null, details ? JSON.stringify(details) : null, isoNow()).run();
}

async function issueDevice(env: Env, userId: string, name: string) {
  const token = randomToken();
  const deviceId = crypto.randomUUID();
  const now = isoNow();
  await env.DB.prepare(`INSERT INTO devices (id, user_id, name, token_hash, status, created_at, last_seen_at)
    VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)`)
    .bind(deviceId, userId, cleanText(name, "Device name", 100), await tokenHash(token, env.TOKEN_PEPPER), now).run();
  return { token, deviceId };
}

type SessionUser = {
  id: string;
  display_name: string;
  role: "owner" | "member";
  status: string;
};

function sessionUser(user: SessionUser) {
  return {
    id: user.id,
    displayName: user.display_name,
    role: user.role,
    status: "active" as const,
  };
}

async function rateBucket(env: Env, value: string) {
  return tokenHash(value, env.TOKEN_PEPPER);
}

async function enforceRateLimit(env: Env, buckets: string[], message = "Too many activation attempts. Try again in a few minutes.") {
  const now = isoNow();
  for (const bucket of buckets) {
    const attempt = await env.DB.prepare("SELECT blocked_until FROM auth_attempts WHERE bucket = ?1")
      .bind(bucket).first<{ blocked_until: string | null }>();
    if (attempt?.blocked_until && attempt.blocked_until > now) {
      throw new HttpError(429, message, "rate_limited");
    }
  }
}

async function recordAttempt(env: Env, bucket: string, maximum: number, windowMilliseconds = 15 * 60_000) {
  const now = isoNow();
  const cutoff = new Date(Date.now() - windowMilliseconds).toISOString();
  const existing = await env.DB.prepare("SELECT failures, window_started_at FROM auth_attempts WHERE bucket = ?1")
    .bind(bucket).first<{ failures: number; window_started_at: string }>();
  const failures = !existing || existing.window_started_at < cutoff ? 1 : Number(existing.failures) + 1;
  const windowStarted = !existing || existing.window_started_at < cutoff ? now : existing.window_started_at;
  const blockedUntil = failures >= maximum ? new Date(Date.now() + windowMilliseconds).toISOString() : null;
  await env.DB.prepare(`INSERT INTO auth_attempts (bucket, failures, window_started_at, blocked_until, updated_at)
    VALUES (?1, ?2, ?3, ?4, ?5)
    ON CONFLICT(bucket) DO UPDATE SET failures = excluded.failures, window_started_at = excluded.window_started_at,
      blocked_until = excluded.blocked_until, updated_at = excluded.updated_at`)
    .bind(bucket, failures, windowStarted, blockedUntil, now).run();
}

export function cleanLicenceKey(value: unknown) {
  if (typeof value !== "string") throw new HttpError(400, "Licence key is required.");
  const key = value.trim();
  if (!/^CE-[A-Za-z0-9_-]{32}$/.test(key)) throw new HttpError(400, "That licence key is not valid.");
  return key;
}

async function activateLicence(request: Request, env: Env) {
  const input = await body<{ licenceKey: string; displayName: string; deviceName: string }>(request);
  const suppliedKey = typeof input.licenceKey === "string" ? input.licenceKey.trim() : "";
  const displayName = cleanText(input.displayName, "Display name", 100);
  const deviceName = cleanText(input.deviceName, "Device name", 100);
  const ip = request.headers.get("CF-Connecting-IP") || "unknown";
  const ipBucket = await rateBucket(env, `licence:ip:${ip}`);
  await enforceRateLimit(env, [ipBucket], "Too many licence attempts. Try again in a few minutes.");
  const suppliedHash = await tokenHash(suppliedKey, env.TOKEN_PEPPER);
  const bootstrapHash = await tokenHash(env.BOOTSTRAP_TOKEN, env.TOKEN_PEPPER);
  const now = isoNow();
  let user: SessionUser;
  let licenceId: string | undefined;
  if (secureEqual(suppliedHash, bootstrapHash)) {
    const existing = await env.DB.prepare(`SELECT id, display_name, role, status FROM users
      WHERE role = 'owner' ORDER BY created_at LIMIT 1`).first<SessionUser>();
    if (existing) {
      await env.DB.prepare(`UPDATE users SET display_name = ?1, status = 'active', updated_at = ?2 WHERE id = ?3`)
        .bind(displayName, now, existing.id).run();
      user = { ...existing, display_name: displayName, status: "active" };
    } else {
      const userId = crypto.randomUUID();
      await env.DB.prepare(`INSERT INTO users (id, email, username, display_name, role, status, created_at, updated_at)
        VALUES (?1, ?2, 'admin', ?3, 'owner', 'active', ?4, ?4)`)
        .bind(userId, internalEmail(userId), displayName, now).run();
      user = { id: userId, display_name: displayName, role: "owner", status: "active" };
    }
  } else {
    const key = cleanLicenceKey(suppliedKey);
    const licence = await env.DB.prepare(`SELECT id, user_id, status FROM licence_keys WHERE token_hash = ?1`)
      .bind(await tokenHash(key, env.TOKEN_PEPPER)).first<{ id: string; user_id: string | null; status: string }>();
    if (!licence || licence.status !== "active") {
      await recordAttempt(env, ipBucket, 10);
      throw new HttpError(403, "That licence key is invalid or has been revoked.", "invalid_licence");
    }
    licenceId = licence.id;
    if (licence.user_id) {
      const existing = await env.DB.prepare("SELECT id, display_name, role, status FROM users WHERE id = ?1")
        .bind(licence.user_id).first<SessionUser>();
      if (!existing) throw new HttpError(409, "This licence is not linked to a valid user.");
      user = existing;
    } else {
      const userId = crypto.randomUUID();
      const results = await env.DB.batch([
        env.DB.prepare(`INSERT INTO users (id, email, username, display_name, role, status, created_at, updated_at)
          VALUES (?1, ?2, NULL, ?3, 'member', 'active', ?4, ?4)`)
          .bind(userId, internalEmail(userId), displayName, now),
        env.DB.prepare(`UPDATE licence_keys SET user_id = ?1, last_activated_at = ?2
          WHERE id = ?3 AND user_id IS NULL`).bind(userId, now, licence.id),
      ]);
      if (results[1].meta.changes !== 1) throw new HttpError(409, "This licence was activated by another device. Try again.");
      user = { id: userId, display_name: displayName, role: "member", status: "active" };
    }
    if (user.status !== "active") throw new HttpError(403, "That licence has been revoked.", "invalid_licence");
    await env.DB.prepare("UPDATE licence_keys SET last_activated_at = ?1 WHERE id = ?2").bind(now, licence.id).run();
  }
  await env.DB.prepare("DELETE FROM auth_attempts WHERE bucket = ?1").bind(ipBucket).run();
  const device = await issueDevice(env, user.id, deviceName);
  await audit(env, user.id, "licence.activate", licenceId ? "licence" : "owner", licenceId || user.id, { deviceId: device.deviceId });
  return json({ token: device.token, deviceId: device.deviceId, user: sessionUser(user) } satisfies DeviceSession, 201);
}

async function logout(env: Env, actor: Principal) {
  await env.DB.prepare("UPDATE devices SET status = 'revoked' WHERE id = ?1").bind(actor.deviceId).run();
  await audit(env, actor.userId, "device.logout", "device", actor.deviceId);
  return json({ loggedOut: true });
}

function licence(row: Record<string, unknown>): Licence {
  return {
    id: String(row.id),
    keyHint: String(row.key_hint),
    label: String(row.label),
    status: String(row.status) as Licence["status"],
    userId: row.user_id ? String(row.user_id) : undefined,
    displayName: row.display_name ? String(row.display_name) : undefined,
    deviceCount: Number(row.device_count || 0),
    activeClipCount: Number(row.active_clip_count || 0),
    activeBytes: Number(row.active_bytes || 0),
    uploadedBytes: Number(row.uploaded_bytes || 0),
    createdAt: String(row.created_at),
    lastActivatedAt: row.last_activated_at ? String(row.last_activated_at) : undefined,
    revokedAt: row.revoked_at ? String(row.revoked_at) : undefined,
  };
}

const licenceSelect = `SELECT licence_keys.*, users.display_name,
  (SELECT COUNT(*) FROM devices WHERE devices.user_id = users.id AND devices.status = 'active') AS device_count,
  (SELECT COUNT(*) FROM clips WHERE clips.owner_id = users.id AND clips.status = 'published' AND clips.expires_at > ?1) AS active_clip_count,
  (SELECT COALESCE(SUM(size), 0) FROM clips WHERE clips.owner_id = users.id AND clips.status = 'published' AND clips.expires_at > ?1) AS active_bytes,
  (SELECT COALESCE(SUM(quantity), 0) FROM usage_events WHERE usage_events.user_id = users.id AND usage_events.type = 'stored_bytes') AS uploaded_bytes
  FROM licence_keys LEFT JOIN users ON users.id = licence_keys.user_id`;

async function listLicences(env: Env, actor: Principal) {
  owner(actor);
  const rows = await env.DB.prepare(`${licenceSelect} ORDER BY licence_keys.created_at DESC`).bind(isoNow()).all<Record<string, unknown>>();
  return json(rows.results.map(licence));
}

async function createLicence(request: Request, env: Env, actor: Principal) {
  owner(actor);
  const input = await body<{ label: string }>(request);
  const label = cleanText(input.label, "Licence label", 100);
  const key = `CE-${randomToken(24)}`;
  const id = crypto.randomUUID();
  const now = isoNow();
  const keyHint = `••••-${key.slice(-6)}`;
  await env.DB.prepare(`INSERT INTO licence_keys (id, token_hash, key_hint, label, status, created_by, created_at)
    VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6)`)
    .bind(id, await tokenHash(key, env.TOKEN_PEPPER), keyHint, label, actor.userId, now).run();
  await audit(env, actor.userId, "licence.create", "licence", id, { label, keyHint });
  return json({ ...licence({ id, key_hint: keyHint, label, status: "active", created_at: now }), key } satisfies CreatedLicence, 201);
}

async function updateLicence(request: Request, env: Env, actor: Principal, id: string) {
  owner(actor);
  const input = await body<{ status: "active" | "revoked" }>(request);
  if (input.status !== "active" && input.status !== "revoked") throw new HttpError(400, "Status must be active or revoked.");
  const existing = await env.DB.prepare("SELECT id, user_id FROM licence_keys WHERE id = ?1").bind(id)
    .first<{ id: string; user_id: string | null }>();
  if (!existing) throw new HttpError(404, "Licence not found.");
  const now = isoNow();
  const statements = [env.DB.prepare(`UPDATE licence_keys SET status = ?1, revoked_at = ?2 WHERE id = ?3`)
    .bind(input.status, input.status === "revoked" ? now : null, id)];
  if (existing.user_id) {
    statements.push(env.DB.prepare("UPDATE users SET status = ?1, updated_at = ?2 WHERE id = ?3")
      .bind(input.status, now, existing.user_id));
    if (input.status === "revoked") {
      statements.push(env.DB.prepare("UPDATE devices SET status = 'revoked' WHERE user_id = ?1").bind(existing.user_id));
    }
  }
  await env.DB.batch(statements);
  await audit(env, actor.userId, `licence.${input.status}`, "licence", id);
  return json({ updated: true });
}

async function temporaryCredentials(env: Env, objects: string[]) {
  const ttlSeconds = 3_600;
  return createR2TemporaryCredentials({
    accountId: env.R2_ACCOUNT_ID,
    accessKeyId: env.R2_PARENT_ACCESS_KEY_ID,
    secretAccessKey: env.R2_PARENT_SECRET_ACCESS_KEY,
    bucket: env.R2_BUCKET_NAME,
    objects,
    ttlSeconds,
  });
}

async function createUpload(request: Request, env: Env, actor: Principal) {
  const intent = validateUploadIntent(await body<UploadIntent>(request), Number(env.MAX_UPLOAD_BYTES || 5_368_709_120));
  const clipId = crypto.randomUUID();
  const uploadId = crypto.randomUUID();
  const slug = randomToken(18);
  const videoKey = `published/${clipId}/video.mp4`;
  const thumbnailKey = `published/${clipId}/thumbnail.jpg`;
  const now = isoNow();
  const uploadExpiresAt = addSeconds(3_600);
  await env.DB.batch([
    env.DB.prepare(`INSERT INTO clips (id, owner_id, slug, title, status, duration, width, height, fps, created_at, updated_at)
      VALUES (?1, ?2, ?3, ?4, 'uploading', ?5, ?6, ?7, ?8, ?9, ?9)`)
      .bind(clipId, actor.userId, slug, intent.title, intent.duration, intent.width, intent.height, intent.fps, now),
    env.DB.prepare(`INSERT INTO assets (id, clip_id, kind, r2_key, expected_size, created_at)
      VALUES (?1, ?2, 'video', ?3, ?4, ?5)`).bind(crypto.randomUUID(), clipId, videoKey, intent.videoSize, now),
    env.DB.prepare(`INSERT INTO assets (id, clip_id, kind, r2_key, expected_size, created_at)
      VALUES (?1, ?2, 'thumbnail', ?3, ?4, ?5)`).bind(crypto.randomUUID(), clipId, thumbnailKey, intent.thumbnailSize, now),
    env.DB.prepare(`INSERT INTO upload_sessions (id, clip_id, device_id, status, expires_at, created_at)
      VALUES (?1, ?2, ?3, 'created', ?4, ?5)`).bind(uploadId, clipId, actor.deviceId, uploadExpiresAt, now),
  ]);
  try {
    const credentials = await temporaryCredentials(env, [videoKey, thumbnailKey]);
    await audit(env, actor.userId, "upload.create", "clip", clipId, { uploadId, videoSize: intent.videoSize });
    return json({ uploadId, clipId, videoKey, thumbnailKey, credentials } satisfies CreatedUpload, 201);
  } catch (error) {
    await env.DB.prepare("UPDATE clips SET status = 'failed', updated_at = ?1 WHERE id = ?2").bind(isoNow(), clipId).run();
    throw error;
  }
}

async function completeUpload(request: Request, env: Env, actor: Principal, uploadId: string) {
  await body<CompleteUpload>(request);
  const upload = await env.DB.prepare(`SELECT upload_sessions.id, upload_sessions.clip_id, upload_sessions.status,
    upload_sessions.expires_at, clips.owner_id FROM upload_sessions JOIN clips ON clips.id = upload_sessions.clip_id
    WHERE upload_sessions.id = ?1`).bind(uploadId).first<Record<string, string>>();
  if (!upload) throw new HttpError(404, "Upload session not found.");
  if (upload.owner_id !== actor.userId && actor.role !== "owner") throw new HttpError(403, "You do not own this upload.");
  if (upload.status === "complete") throw new HttpError(409, "This upload has already been completed.");
  if (upload.expires_at <= isoNow()) throw new HttpError(410, "This upload session has expired.");
  const assets = await env.DB.prepare("SELECT id, kind, r2_key, expected_size FROM assets WHERE clip_id = ?1")
    .bind(upload.clip_id).all<Record<string, string | number>>();
  if (assets.results.length !== 2) throw new HttpError(409, "Upload assets are incomplete.");
  let totalSize = 0;
  for (const asset of assets.results) {
    const object = await env.MEDIA.head(String(asset.r2_key));
    if (!object || object.size !== Number(asset.expected_size)) {
      throw new HttpError(409, `${asset.kind} has not finished uploading or has the wrong size.`);
    }
    totalSize += object.size;
    await env.DB.prepare("UPDATE assets SET actual_size = ?1, etag = ?2 WHERE id = ?3")
      .bind(object.size, object.etag, asset.id).run();
  }
  const now = isoNow();
  const expiresAt = addDays(Number(env.CLIP_TTL_DAYS || 30));
  await env.DB.batch([
    env.DB.prepare(`UPDATE clips SET status = 'published', size = ?1, published_at = ?2, expires_at = ?3, updated_at = ?2 WHERE id = ?4`)
      .bind(totalSize, now, expiresAt, upload.clip_id),
    env.DB.prepare("UPDATE upload_sessions SET status = 'complete', completed_at = ?1 WHERE id = ?2").bind(now, uploadId),
    env.DB.prepare("INSERT INTO usage_events (id, user_id, type, quantity, created_at) VALUES (?1, ?2, 'stored_bytes', ?3, ?4)")
      .bind(crypto.randomUUID(), upload.owner_id, totalSize, now),
  ]);
  await audit(env, actor.userId, "upload.complete", "clip", upload.clip_id, { totalSize });
  return json({ complete: true, expiresAt });
}

function cloudClip(row: Record<string, unknown>, env: Env): CloudClip {
  const videoKey = String(row.video_key || "");
  const thumbnailKey = String(row.thumbnail_key || "");
  const slug = String(row.slug);
  return {
    id: String(row.id), ownerId: String(row.owner_id), ownerName: String(row.owner_name), slug,
    title: String(row.title), status: String(row.status) as CloudClip["status"],
    publishedAt: row.published_at ? String(row.published_at) : undefined,
    expiresAt: row.expires_at ? String(row.expires_at) : undefined,
    duration: Number(row.duration), width: Number(row.width), height: Number(row.height), fps: Number(row.fps), size: Number(row.size),
    url: `${env.APP_BASE_URL}/c/${encodeURIComponent(slug)}`,
    mediaUrl: videoKey ? `${env.MEDIA_BASE_URL}/${videoKey.split("/").map(encodeURIComponent).join("/")}` : undefined,
    thumbnailUrl: thumbnailKey ? `${env.MEDIA_BASE_URL}/${thumbnailKey.split("/").map(encodeURIComponent).join("/")}` : undefined,
  };
}

const clipSelect = `SELECT clips.*, users.display_name AS owner_name,
  MAX(CASE WHEN assets.kind = 'video' THEN assets.r2_key END) AS video_key,
  MAX(CASE WHEN assets.kind = 'thumbnail' THEN assets.r2_key END) AS thumbnail_key
  FROM clips JOIN users ON users.id = clips.owner_id LEFT JOIN assets ON assets.clip_id = clips.id`;

async function listClips(env: Env) {
  const result = await env.DB.prepare(`${clipSelect} WHERE clips.status = 'published' AND clips.expires_at > ?1 GROUP BY clips.id ORDER BY clips.created_at DESC LIMIT 1000`).bind(isoNow()).all();
  return json(result.results.map((row) => cloudClip(row, env)));
}

async function deleteClip(env: Env, actor: Principal, clipId: string) {
  const clip = await env.DB.prepare("SELECT owner_id, status FROM clips WHERE id = ?1").bind(clipId).first<{ owner_id: string; status: string }>();
  if (!clip) throw new HttpError(404, "Clip not found.");
  if (clip.owner_id !== actor.userId && actor.role !== "owner") throw new HttpError(403, "You cannot delete this clip.");
  const assets = await env.DB.prepare("SELECT r2_key FROM assets WHERE clip_id = ?1").bind(clipId).all<{ r2_key: string }>();
  if (assets.results.length) await env.MEDIA.delete(assets.results.map((asset) => asset.r2_key));
  await env.DB.prepare("UPDATE clips SET status = 'deleted', updated_at = ?1 WHERE id = ?2").bind(isoNow(), clipId).run();
  await audit(env, actor.userId, "clip.delete", "clip", clipId);
  return json({ deleted: true });
}

async function extendClip(env: Env, actor: Principal, clipId: string) {
  const clip = await env.DB.prepare("SELECT owner_id, status FROM clips WHERE id = ?1").bind(clipId).first<{ owner_id: string; status: string }>();
  if (!clip) throw new HttpError(404, "Clip not found.");
  if (clip.owner_id !== actor.userId && actor.role !== "owner") throw new HttpError(403, "You cannot extend this clip.");
  if (clip.status !== "published") throw new HttpError(409, "Only an active published clip can be extended.");
  const assets = await env.DB.prepare("SELECT r2_key FROM assets WHERE clip_id = ?1").bind(clipId).all<{ r2_key: string }>();
  for (const asset of assets.results) {
    const object = await env.MEDIA.get(asset.r2_key);
    if (!object) throw new HttpError(409, "One or more clip assets have already expired.");
    await env.MEDIA.put(asset.r2_key, object.body, { httpMetadata: object.httpMetadata, customMetadata: object.customMetadata });
  }
  const expiresAt = addDays(Number(env.CLIP_TTL_DAYS || 30));
  await env.DB.prepare("UPDATE clips SET expires_at = ?1, updated_at = ?2 WHERE id = ?3").bind(expiresAt, isoNow(), clipId).run();
  await audit(env, actor.userId, "clip.extend", "clip", clipId, { expiresAt });
  return json({ extended: true, expiresAt });
}

async function publicShare(env: Env, slug: string) {
  const row = await env.DB.prepare(`${clipSelect} WHERE clips.slug = ?1 GROUP BY clips.id`).bind(slug).first<Record<string, unknown>>();
  if (!row) return new Response("Not found", { status: 404 });
  if (row.status !== "published" || !row.expires_at || String(row.expires_at) <= isoNow()) {
    return new Response(gonePage(), { status: 410, headers: { "content-type": "text/html; charset=utf-8", "cache-control": "public, max-age=60" } });
  }
  const clip = cloudClip(row, env);
  const mediaOrigin = new URL(env.MEDIA_BASE_URL).origin;
  return new Response(sharePage({
    title: clip.title,
    siteName: "Dabs Clip Engine",
    uploaderName: clip.ownerName,
    pageUrl: clip.url!,
    mediaUrl: clip.mediaUrl!,
    thumbnailUrl: clip.thumbnailUrl!,
    duration: clip.duration,
    width: clip.width,
    height: clip.height,
    fps: clip.fps,
    publishedAt: clip.publishedAt || String(row.created_at),
    expiresAt: clip.expiresAt!,
  }), { headers: {
    "content-type": "text/html; charset=utf-8",
    "cache-control": "public, max-age=300",
    "content-security-policy": `default-src 'none'; img-src ${mediaOrigin}; media-src ${mediaOrigin}; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`,
    "referrer-policy": "no-referrer",
    "x-frame-options": "DENY",
  } });
}

async function route(request: Request, env: Env) {
  const url = new URL(request.url);
  const path = url.pathname.replace(/\/+$/, "") || "/";
  if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: {
    "access-control-allow-methods": "GET,POST,PATCH,DELETE,OPTIONS",
    "access-control-allow-headers": "authorization,content-type",
    "access-control-max-age": "86400",
  } });
  if (request.method === "GET" && path === "/health") return json({ status: "ok", version: "1.0.0" });
  const shareMatch = path.match(/^\/c\/([A-Za-z0-9_-]+)$/);
  if (request.method === "GET" && shareMatch) return publicShare(env, shareMatch[1]);
  if (request.method === "POST" && path === "/v1/auth/licence") return activateLicence(request, env);

  const actor = await principal(request, env);
  if (request.method === "GET" && path === "/v1/me") return json({
    id: actor.userId,
    displayName: actor.displayName,
    role: actor.role,
    status: "active",
  });
  if (request.method === "POST" && path === "/v1/auth/logout") return logout(env, actor);
  if (request.method === "GET" && path === "/v1/licences") return listLicences(env, actor);
  if (request.method === "POST" && path === "/v1/licences") return createLicence(request, env, actor);
  const licenceMatch = path.match(/^\/v1\/licences\/([0-9a-f-]+)$/i);
  if (request.method === "PATCH" && licenceMatch) return updateLicence(request, env, actor, licenceMatch[1]);
  if (request.method === "POST" && path === "/v1/uploads") return createUpload(request, env, actor);
  const completeMatch = path.match(/^\/v1\/uploads\/([0-9a-f-]+)\/complete$/i);
  if (request.method === "POST" && completeMatch) return completeUpload(request, env, actor, completeMatch[1]);
  if (request.method === "GET" && path === "/v1/clips") return listClips(env);
  const extendMatch = path.match(/^\/v1\/clips\/([0-9a-f-]+)\/extend$/i);
  if (request.method === "POST" && extendMatch) return extendClip(env, actor, extendMatch[1]);
  const clipMatch = path.match(/^\/v1\/clips\/([0-9a-f-]+)$/i);
  if (request.method === "DELETE" && clipMatch) return deleteClip(env, actor, clipMatch[1]);
  throw new HttpError(404, "Endpoint not found.");
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    try {
      return cors(request, await route(request, env));
    } catch (error) {
      return cors(request, errorResponse(error));
    }
  },
  async scheduled(_event: ScheduledController, env: Env, context: ExecutionContext) {
    context.waitUntil((async () => {
      const now = isoNow();
      const expired = await env.DB.prepare(`UPDATE clips SET status = 'expired', updated_at = ?1
        WHERE status = 'published' AND expires_at <= ?1 RETURNING id`).bind(now).all<{ id: string }>();
      for (const clip of expired.results) await audit(env, null, "clip.expire", "clip", clip.id);
      await env.DB.prepare("UPDATE upload_sessions SET status = 'expired' WHERE status IN ('created', 'uploading') AND expires_at <= ?1").bind(now).run();
    })());
  },
} satisfies ExportedHandler<Env>;
