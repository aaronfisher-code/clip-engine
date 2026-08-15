import type { ApiError } from "@clip-engine/contracts";

export class HttpError extends Error {
  constructor(public status: number, message: string, public code?: string) {
    super(message);
  }
}

export function json(value: unknown, status = 200, headers: HeadersInit = {}) {
  return Response.json(value, { status, headers: { "cache-control": "no-store", ...headers } });
}

export async function body<T>(request: Request): Promise<T> {
  const contentType = request.headers.get("content-type") || "";
  if (!contentType.toLowerCase().startsWith("application/json")) throw new HttpError(415, "Expected a JSON request body.");
  try {
    return await request.json<T>();
  } catch {
    throw new HttpError(400, "The JSON request body is invalid.");
  }
}

export function errorResponse(error: unknown) {
  if (error instanceof HttpError) return json({ error: error.message, code: error.code } satisfies ApiError, error.status);
  console.error(error);
  return json({ error: "An unexpected server error occurred." } satisfies ApiError, 500);
}

export function cors(request: Request, response: Response) {
  const origin = request.headers.get("origin");
  const headers = new Headers(response.headers);
  headers.set("x-content-type-options", "nosniff");
  headers.set("referrer-policy", "no-referrer");
  headers.set("permissions-policy", "camera=(), microphone=(), geolocation=()");
  if (origin && (origin === "tauri://localhost" || origin === "http://tauri.localhost" || origin.startsWith("http://localhost:"))) {
    headers.set("access-control-allow-origin", origin);
    headers.set("vary", "origin");
  }
  return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
}
