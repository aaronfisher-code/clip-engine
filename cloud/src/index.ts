import type {
  CloudClip,
  CompleteUpload,
  AccessRequest,
  CreatedAccessRequest,
  CreatedInvite,
  CreatedUpload,
  DeviceSession,
  UploadIntent,
  UserRole,
} from "@clip-engine/contracts";
import { authenticate, bearerToken, randomToken, secureEqual, tokenHash } from "./security";
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

export function cleanUsername(value: unknown) {
  if (typeof value !== "string") throw new HttpError(400, "Username is required.");
  const username = value.trim().toLowerCase();
  if (!/^[a-z0-9][a-z0-9_-]{2,31}$/.test(username)) {
    throw new HttpError(400, "Username must be 3–32 characters using letters, numbers, hyphens, or underscores.");
  }
  return username;
}

export function cleanCredential(value: unknown) {
  if (typeof value !== "string" || !/^[A-Za-z0-9_-]{43}$/.test(value)) {
    throw new HttpError(400, "The password credential is invalid.");
  }
  return value;
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

type PasswordUser = {
  id: string;
  username: string | null;
  password_hash: string | null;
  password_salt: string | null;
  password_iterations: number | null;
  password_scheme: "server-pbkdf2-v1" | "client-pbkdf2-v1";
  display_name: string;
  role: UserRole;
  status: string;
};

function sessionUser(user: PasswordUser) {
  return {
    id: user.id,
    username: user.username || "",
    displayName: user.display_name,
    role: user.role,
    status: "active" as const,
  };
}

async function ensureUsernameAvailable(env: Env, username: string, exceptUserId?: string) {
  const existing = await env.DB.prepare("SELECT id FROM users WHERE username = ?1 COLLATE NOCASE")
    .bind(username).first<{ id: string }>();
  if (existing && existing.id !== exceptUserId) throw new HttpError(409, "That username is already in use.");
}

async function rateBucket(env: Env, value: string) {
  return tokenHash(value, env.TOKEN_PEPPER);
}

async function enforceRateLimit(env: Env, buckets: string[], message = "Too many sign-in attempts. Try again in a few minutes.") {
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

async function login(request: Request, env: Env) {
  const input = await body<{ username: string; credentialSecret: string; ownerToken?: string; deviceName: string }>(request);
  const username = cleanUsername(input.username);
  const credentialSecret = cleanCredential(input.credentialSecret);
  const ip = request.headers.get("CF-Connecting-IP") || "unknown";
  const usernameBucket = await rateBucket(env, `login:username:${username}`);
  const ipBucket = await rateBucket(env, `login:ip:${ip}`);
  await enforceRateLimit(env, [usernameBucket, ipBucket]);
  let user = await env.DB.prepare(`SELECT id, username, password_hash, password_salt, password_iterations,
    password_scheme, display_name, role, status FROM users WHERE username = ?1 COLLATE NOCASE`).bind(username).first<PasswordUser>();
  const submittedHash = await tokenHash(credentialSecret, env.TOKEN_PEPPER);
  let valid = user?.password_scheme === "client-pbkdf2-v1"
    && secureEqual(submittedHash, user.password_hash || "0".repeat(64));
  if (!valid && username === "admin" && typeof input.ownerToken === "string") {
    const suppliedOwnerToken = await tokenHash(input.ownerToken, env.TOKEN_PEPPER);
    const expectedOwnerToken = await tokenHash(env.BOOTSTRAP_TOKEN, env.TOKEN_PEPPER);
    if (secureEqual(suppliedOwnerToken, expectedOwnerToken)) {
      const existingOwner = await env.DB.prepare(`SELECT id, username, password_hash, password_salt, password_iterations,
        password_scheme, display_name, role, status FROM users WHERE role = 'owner' ORDER BY created_at LIMIT 1`).first<PasswordUser>();
      const now = isoNow();
      if (existingOwner) {
        if (existingOwner.status !== "active") throw new HttpError(403, "The owner account is not active.");
        await ensureUsernameAvailable(env, "admin", existingOwner.id);
        await env.DB.prepare(`UPDATE users SET username = 'admin', password_hash = ?1, password_salt = NULL,
          password_iterations = 600000, password_scheme = 'client-pbkdf2-v1', display_name = 'Admin', updated_at = ?2
          WHERE id = ?3`).bind(submittedHash, now, existingOwner.id).run();
        user = { ...existingOwner, username: "admin", password_hash: submittedHash, password_salt: null,
          password_iterations: 600_000, password_scheme: "client-pbkdf2-v1", display_name: "Admin" };
      } else {
        const userId = crypto.randomUUID();
        await env.DB.prepare(`INSERT INTO users (id, email, username, password_hash, password_salt, password_iterations,
          password_scheme, display_name, role, status, created_at, updated_at)
          VALUES (?1, ?2, 'admin', ?3, NULL, 600000, 'client-pbkdf2-v1', 'Admin', 'owner', 'active', ?4, ?4)`)
          .bind(userId, internalEmail(userId), submittedHash, now).run();
        user = { id: userId, username: "admin", password_hash: submittedHash, password_salt: null,
          password_iterations: 600_000, password_scheme: "client-pbkdf2-v1", display_name: "Admin", role: "owner", status: "active" };
      }
      valid = true;
    }
  }
  if (!user || !valid || user.status !== "active") {
    await Promise.all([recordAttempt(env, usernameBucket, 5), recordAttempt(env, ipBucket, 25)]);
    throw new HttpError(401, "The username or password is incorrect.", "invalid_credentials");
  }
  await env.DB.prepare("DELETE FROM auth_attempts WHERE bucket = ?1").bind(usernameBucket).run();
  const device = await issueDevice(env, user.id, input.deviceName);
  await audit(env, user.id, "auth.login", "device", device.deviceId);
  return json({ token: device.token, deviceId: device.deviceId, user: sessionUser(user) } satisfies DeviceSession, 201);
}

async function logout(env: Env, actor: Principal) {
  await env.DB.prepare("UPDATE devices SET status = 'revoked' WHERE id = ?1").bind(actor.deviceId).run();
  await audit(env, actor.userId, "device.logout", "device", actor.deviceId);
  return json({ loggedOut: true });
}

async function redeemInvite(request: Request, env: Env) {
  const input = await body<{ inviteToken: string; username: string; credentialSecret: string; displayName?: string; deviceName: string }>(request);
  const hash = await tokenHash(input.inviteToken || "", env.TOKEN_PEPPER);
  const invite = await env.DB.prepare(`SELECT id, username, role, purpose, target_user_id, expires_at FROM invites
    WHERE token_hash = ?1 AND redeemed_at IS NULL`).bind(hash).first<Record<string, string>>();
  if (!invite || !invite.username || invite.expires_at <= isoNow()) {
    throw new HttpError(403, "This invitation is invalid or has expired.");
  }
  const username = cleanUsername(input.username);
  if (username !== invite.username.toLowerCase()) throw new HttpError(403, "This invitation does not match that username.");
  const credentialHash = await tokenHash(cleanCredential(input.credentialSecret), env.TOKEN_PEPPER);
  const now = isoNow();
  let user: PasswordUser;
  if (invite.purpose === "password_reset") {
    user = await env.DB.prepare(`SELECT id, username, password_hash, password_salt, password_iterations,
      password_scheme, display_name, role, status FROM users WHERE id = ?1 AND username = ?2 COLLATE NOCASE`)
      .bind(invite.target_user_id, invite.username).first<PasswordUser>() as PasswordUser;
    if (!user || user.status !== "active") throw new HttpError(403, "This invitation is invalid or has expired.");
    const displayName = input.displayName ? cleanText(input.displayName, "Display name", 100) : user.display_name;
    const results = await env.DB.batch([
      env.DB.prepare("UPDATE invites SET redeemed_at = ?1, redeemed_by = ?2 WHERE id = ?3 AND redeemed_at IS NULL").bind(now, user.id, invite.id),
      env.DB.prepare(`UPDATE users SET password_hash = ?1, password_salt = NULL, password_iterations = 600000,
        password_scheme = 'client-pbkdf2-v1', display_name = ?2, updated_at = ?3 WHERE id = ?4`)
        .bind(credentialHash, displayName, now, user.id),
      env.DB.prepare("UPDATE devices SET status = 'revoked' WHERE user_id = ?1").bind(user.id),
    ]);
    if (results[0].meta.changes !== 1) throw new HttpError(409, "This invitation has already been redeemed.");
    user = { ...user, display_name: displayName };
  } else {
    const displayName = cleanText(input.displayName, "Display name", 100);
    await ensureUsernameAvailable(env, username);
    const userId = crypto.randomUUID();
    const results = await env.DB.batch([
      env.DB.prepare("UPDATE invites SET redeemed_at = ?1 WHERE id = ?2 AND redeemed_at IS NULL").bind(now, invite.id),
      env.DB.prepare(`INSERT INTO users (id, email, username, password_hash, password_salt, password_iterations,
        password_scheme, display_name, role, status, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, NULL, 600000, 'client-pbkdf2-v1', ?5, ?6, 'active', ?7, ?7)`)
        .bind(userId, internalEmail(userId), username, credentialHash, displayName, invite.role, now),
    ]);
    if (results[0].meta.changes !== 1) throw new HttpError(409, "This invitation has already been redeemed.");
    await env.DB.prepare("UPDATE invites SET redeemed_by = ?1 WHERE id = ?2").bind(userId, invite.id).run();
    user = { id: userId, username, password_hash: credentialHash, password_salt: null,
      password_iterations: 600_000, password_scheme: "client-pbkdf2-v1",
      display_name: displayName, role: invite.role as UserRole, status: "active" };
  }
  const device = await issueDevice(env, user.id, input.deviceName);
  await audit(env, user.id, `invite.${invite.purpose}`, "invite", invite.id, { deviceId: device.deviceId });
  return json({ token: device.token, deviceId: device.deviceId, user: sessionUser(user) } satisfies DeviceSession, 201);
}

async function validatePasswordReset(request: Request, env: Env) {
  const input = await body<{ inviteToken: string; username: string }>(request);
  const username = cleanUsername(input.username);
  const hash = await tokenHash(input.inviteToken || "", env.TOKEN_PEPPER);
  const invite = await env.DB.prepare(`SELECT invites.id FROM invites JOIN users ON users.id = invites.target_user_id
    WHERE invites.token_hash = ?1 AND invites.purpose = 'password_reset' AND invites.redeemed_at IS NULL
      AND invites.expires_at > ?2 AND users.username = ?3 COLLATE NOCASE AND users.status = 'active'`)
    .bind(hash, isoNow(), username).first();
  if (!invite) throw new HttpError(403, "That password-reset token is invalid or has expired.");
  return json({ valid: true });
}

function accessRequest(row: Record<string, unknown>): AccessRequest {
  return {
    id: String(row.id),
    username: String(row.username),
    displayName: String(row.display_name),
    status: String(row.status) as AccessRequest["status"],
    createdAt: String(row.created_at),
    reviewedAt: row.reviewed_at ? String(row.reviewed_at) : undefined,
  };
}

async function requestAccess(request: Request, env: Env) {
  const input = await body<{ username: string; displayName: string; credentialSecret: string }>(request);
  const ip = request.headers.get("CF-Connecting-IP") || "unknown";
  const requestBucket = await rateBucket(env, `access-request:ip:${ip}`);
  await enforceRateLimit(env, [requestBucket], "Too many account requests were created from this connection. Try again tomorrow.");
  const username = cleanUsername(input.username);
  if (username === "admin") throw new HttpError(409, "The admin username is reserved for the owner.");
  await ensureUsernameAvailable(env, username);
  const existingRequest = await env.DB.prepare(`SELECT id FROM access_requests WHERE username = ?1 COLLATE NOCASE
    AND status IN ('pending', 'approved')`).bind(username).first();
  if (existingRequest) throw new HttpError(409, "That username already has an account request.");
  const displayName = cleanText(input.displayName, "Display name", 100);
  const credentialHash = await tokenHash(cleanCredential(input.credentialSecret), env.TOKEN_PEPPER);
  const requestToken = randomToken();
  const requestId = crypto.randomUUID();
  const now = isoNow();
  try {
    await env.DB.prepare(`INSERT INTO access_requests (id, invite_id, username, display_name, credential_hash,
      request_token_hash, status, created_at) VALUES (?1, NULL, ?2, ?3, ?4, ?5, 'pending', ?6)`)
      .bind(requestId, username, displayName, credentialHash, await tokenHash(requestToken, env.TOKEN_PEPPER), now).run();
  } catch {
    throw new HttpError(409, "That username already has an account request.");
  }
  await recordAttempt(env, requestBucket, 5, 24 * 60 * 60_000);
  await audit(env, null, "access_request.create", "access_request", requestId, { username });
  return json({ ...accessRequest({ id: requestId, username, display_name: displayName, status: "pending", created_at: now }),
    requestToken } satisfies CreatedAccessRequest, 201);
}

async function accessRequestStatus(request: Request, env: Env) {
  const token = bearerToken(request);
  if (!token) throw new HttpError(401, "The account request token is missing.");
  const hash = await tokenHash(token, env.TOKEN_PEPPER);
  const row = await env.DB.prepare(`SELECT id, username, display_name, status, created_at, reviewed_at
    FROM access_requests WHERE request_token_hash = ?1`).bind(hash).first<Record<string, unknown>>();
  if (!row) throw new HttpError(401, "The account request is no longer available.");
  return json(accessRequest(row));
}

async function listAccessRequests(env: Env, actor: Principal) {
  owner(actor);
  const rows = await env.DB.prepare(`SELECT id, username, display_name, status, created_at, reviewed_at
    FROM access_requests ORDER BY CASE status WHEN 'pending' THEN 0 ELSE 1 END, created_at DESC`).all<Record<string, unknown>>();
  return json(rows.results.map(accessRequest));
}

async function reviewAccessRequest(request: Request, env: Env, actor: Principal, id: string) {
  owner(actor);
  const input = await body<{ decision: "approved" | "denied" }>(request);
  if (input.decision !== "approved" && input.decision !== "denied") throw new HttpError(400, "Decision must be approved or denied.");
  const pending = await env.DB.prepare(`SELECT id, username, display_name, credential_hash, status
    FROM access_requests WHERE id = ?1`).bind(id).first<Record<string, string>>();
  if (!pending) throw new HttpError(404, "Account request not found.");
  if (pending.status !== "pending") throw new HttpError(409, "This account request has already been reviewed.");
  const now = isoNow();
  if (input.decision === "denied") {
    const result = await env.DB.prepare(`UPDATE access_requests SET status = 'denied', reviewed_at = ?1, reviewed_by = ?2
      WHERE id = ?3 AND status = 'pending'`).bind(now, actor.userId, id).run();
    if (result.meta.changes !== 1) throw new HttpError(409, "This account request has already been reviewed.");
  } else {
    const username = cleanUsername(pending.username);
    await ensureUsernameAvailable(env, username);
    const userId = crypto.randomUUID();
    const results = await env.DB.batch([
      env.DB.prepare(`INSERT INTO users (id, email, username, password_hash, password_salt, password_iterations,
        password_scheme, display_name, role, status, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, NULL, 600000, 'client-pbkdf2-v1', ?5, 'member', 'active', ?6, ?6)`)
        .bind(userId, internalEmail(userId), username, pending.credential_hash, pending.display_name, now),
      env.DB.prepare(`UPDATE access_requests SET status = 'approved', user_id = ?1, reviewed_at = ?2, reviewed_by = ?3
        WHERE id = ?4 AND status = 'pending'`).bind(userId, now, actor.userId, id),
    ]);
    if (results[1].meta.changes !== 1) throw new HttpError(409, "This account request has already been reviewed.");
  }
  await audit(env, actor.userId, `access_request.${input.decision}`, "access_request", id, { username: pending.username });
  return json({ updated: true });
}

async function createPasswordReset(env: Env, actor: Principal, id: string) {
  owner(actor);
  const user = await env.DB.prepare("SELECT id, username, role, status FROM users WHERE id = ?1").bind(id)
    .first<{ id: string; username: string | null; role: UserRole; status: string }>();
  if (!user?.username) throw new HttpError(404, "User not found or has no username.");
  if (user.status !== "active") throw new HttpError(409, "Restore this account before resetting its password.");
  const inviteId = crypto.randomUUID();
  const token = randomToken();
  const expiresAt = addDays(1);
  await env.DB.prepare(`INSERT INTO invites (id, email, username, token_hash, role, purpose, target_user_id,
    expires_at, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'password_reset', ?6, ?7, ?8, ?9)`)
    .bind(inviteId, internalEmail(inviteId), user.username, await tokenHash(token, env.TOKEN_PEPPER), user.role,
      user.id, expiresAt, actor.userId, isoNow()).run();
  await audit(env, actor.userId, "password_reset.create", "user", user.id);
  return json({ id: inviteId, username: user.username, role: user.role, purpose: "password_reset", expiresAt, token,
    url: `${env.APP_BASE_URL}/invite/${token}` } satisfies CreatedInvite, 201);
}

async function listUsers(env: Env, actor: Principal) {
  owner(actor);
  const result = await env.DB.prepare(`SELECT users.id, users.username, users.display_name, users.role, users.status,
    (SELECT COUNT(*) FROM devices WHERE devices.user_id = users.id AND devices.status = 'active') AS device_count,
    (SELECT MAX(last_seen_at) FROM devices WHERE devices.user_id = users.id) AS last_seen_at,
    (SELECT COUNT(*) FROM clips WHERE clips.owner_id = users.id AND clips.status = 'published' AND clips.expires_at > ?1) AS active_clip_count,
    (SELECT COALESCE(SUM(size), 0) FROM clips WHERE clips.owner_id = users.id AND clips.status = 'published' AND clips.expires_at > ?1) AS active_bytes,
    (SELECT COALESCE(SUM(quantity), 0) FROM usage_events WHERE usage_events.user_id = users.id AND usage_events.type = 'stored_bytes') AS uploaded_bytes
    FROM users ORDER BY users.created_at`).bind(isoNow()).all();
  return json(result.results.map((row) => ({
    id: String(row.id),
    username: row.username ? String(row.username) : "Not configured",
    displayName: String(row.display_name),
    role: String(row.role),
    status: String(row.status),
    deviceCount: Number(row.device_count || 0),
    lastSeenAt: row.last_seen_at ? String(row.last_seen_at) : undefined,
    activeClipCount: Number(row.active_clip_count || 0),
    activeBytes: Number(row.active_bytes || 0),
    uploadedBytes: Number(row.uploaded_bytes || 0),
  })));
}

async function updateUser(request: Request, env: Env, actor: Principal, id: string) {
  owner(actor);
  const input = await body<{ status: "active" | "revoked" }>(request);
  if (input.status !== "active" && input.status !== "revoked") throw new HttpError(400, "Status must be active or revoked.");
  if (id === actor.userId && input.status === "revoked") throw new HttpError(409, "You cannot revoke your current owner account.");
  const now = isoNow();
  const statements = [env.DB.prepare("UPDATE users SET status = ?1, updated_at = ?2 WHERE id = ?3").bind(input.status, now, id)];
  if (input.status === "revoked") statements.push(env.DB.prepare("UPDATE devices SET status = 'revoked' WHERE user_id = ?1").bind(id));
  await env.DB.batch(statements);
  await audit(env, actor.userId, `user.${input.status}`, "user", id);
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

async function inviteLanding(env: Env, token: string) {
  const invite = await env.DB.prepare(`SELECT username, purpose, expires_at, redeemed_at FROM invites WHERE token_hash = ?1`)
    .bind(await tokenHash(token, env.TOKEN_PEPPER)).first<{ username: string | null; purpose: string; expires_at: string; redeemed_at: string | null }>();
  const available = invite && !invite.redeemed_at && invite.expires_at > isoNow();
  const reset = available && invite.purpose === "password_reset";
  const heading = !available ? "This private link is no longer available" : reset ? "Reset your Clip Engine password" : "Account requests have changed";
  const instructions = !available
    ? "Ask the owner to create another link."
    : reset
      ? `Open Clip Engine, choose <strong>Sign in</strong>, then <strong>Forgot my password</strong>. Enter the username <strong>@${invite.username}</strong> and paste this entire link.`
      : "Open Clip Engine and choose <strong>Create account</strong>. Join links are no longer required; your request will wait for owner approval before you can publish.";
  return new Response(`<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
    <title>Clip Engine invitation</title><style>html{color-scheme:dark}body{margin:0;min-height:100vh;display:grid;place-items:center;background:#090a0d;color:#e5e7eb;font:16px system-ui}main{width:min(520px,calc(100% - 48px));padding:32px;border:1px solid #343842;border-radius:14px;background:#111319;box-shadow:0 24px 80px #0008}b{color:#c7ff3d}p{color:#a4a8b1;line-height:1.6}code{display:block;padding:14px;border-radius:8px;background:#08090b;color:#d3ff63;overflow-wrap:anywhere}</style>
    <main><b>Private ${reset ? "password-reset" : "join"} link</b><h1>${heading}</h1><p>${instructions}</p>${available ? `<code>${env.APP_BASE_URL}/invite/${token}</code>` : ""}</main></html>`, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'",
      "x-frame-options": "DENY",
    },
  });
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
  const inviteMatch = path.match(/^\/invite\/([A-Za-z0-9_-]+)$/);
  if (request.method === "GET" && inviteMatch) return inviteLanding(env, inviteMatch[1]);
  if (request.method === "POST" && path === "/v1/auth/login") return login(request, env);
  if (request.method === "POST" && path === "/v1/auth/redeem") return redeemInvite(request, env);
  if (request.method === "POST" && path === "/v1/auth/password-reset/validate") return validatePasswordReset(request, env);
  if (request.method === "POST" && path === "/v1/access-requests") return requestAccess(request, env);
  if (request.method === "GET" && path === "/v1/access-requests/me") return accessRequestStatus(request, env);

  const actor = await principal(request, env);
  if (request.method === "GET" && path === "/v1/me") return json({
    id: actor.userId,
    username: actor.username,
    displayName: actor.displayName,
    role: actor.role,
    status: "active",
  });
  if (request.method === "POST" && path === "/v1/auth/logout") return logout(env, actor);
  if (request.method === "GET" && path === "/v1/access-requests") return listAccessRequests(env, actor);
  const requestMatch = path.match(/^\/v1\/access-requests\/([0-9a-f-]+)$/i);
  if (request.method === "PATCH" && requestMatch) return reviewAccessRequest(request, env, actor, requestMatch[1]);
  if (request.method === "GET" && path === "/v1/users") return listUsers(env, actor);
  const userMatch = path.match(/^\/v1\/users\/([0-9a-f-]+)$/i);
  if (request.method === "PATCH" && userMatch) return updateUser(request, env, actor, userMatch[1]);
  const resetMatch = path.match(/^\/v1\/users\/([0-9a-f-]+)\/password-reset$/i);
  if (request.method === "POST" && resetMatch) return createPasswordReset(env, actor, resetMatch[1]);
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
