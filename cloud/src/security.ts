import type { Env, Principal } from "./types";

const encoder = new TextEncoder();

export function randomToken(bytes = 32) {
  const value = crypto.getRandomValues(new Uint8Array(bytes));
  return btoa(String.fromCharCode(...value)).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

export async function tokenHash(token: string, pepper: string) {
  const digest = await crypto.subtle.digest("SHA-256", encoder.encode(`${pepper}:${token}`));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function secureEqual(left: string, right: string) {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) {
    difference |= left.charCodeAt(index) ^ right.charCodeAt(index);
  }
  return difference === 0;
}

export function bearerToken(request: Request) {
  const match = request.headers.get("authorization")?.match(/^Bearer\s+(.+)$/i);
  return match?.[1];
}

export async function authenticate(request: Request, env: Env): Promise<Principal | null> {
  const token = bearerToken(request);
  if (!token) return null;
  const hash = await tokenHash(token, env.TOKEN_PEPPER);
  const row = await env.DB.prepare(`
    SELECT users.id AS user_id, devices.id AS device_id, devices.last_seen_at, users.username, users.display_name, users.role
    FROM devices JOIN users ON users.id = devices.user_id
    WHERE devices.token_hash = ?1 AND devices.status = 'active' AND users.status = 'active'
  `).bind(hash).first<Record<string, string>>();
  if (!row) return null;
  const now = new Date().toISOString();
  const seenCutoff = new Date(Date.now() - 15 * 60_000).toISOString();
  if (row.last_seen_at < seenCutoff) {
    await env.DB.prepare("UPDATE devices SET last_seen_at = ?1 WHERE id = ?2").bind(now, row.device_id).run();
  }
  return {
    userId: row.user_id,
    deviceId: row.device_id,
    username: row.username || "",
    displayName: row.display_name,
    role: row.role as Principal["role"],
  };
}
