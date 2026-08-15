import type { AppConfig, Clip, Job } from "./types";

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init);
  const body = await response.json().catch(() => null);
  if (!response.ok) throw new Error(body?.error || `Request failed (${response.status})`);
  return body as T;
}

export const api = {
  config: () => request<AppConfig>("/api/config"),
  clips: () => request<Clip[]>("/api/clips"),
  jobs: () => request<Job[]>("/api/jobs"),
  scan: () => request<{ count: number; clips: Clip[] }>("/api/clips/scan", { method: "POST" }),
  remove: (clipId: string) => request<{ deleted: boolean; removedFileCount: number }>(`/api/clips/${clipId}`, { method: "DELETE" }),
  publish: (clipId: string, start: number, end: number, audioStreamIndexes: number[]) =>
    request<Job>(`/api/clips/${clipId}/publish`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ start, end, audioStreamIndexes }),
    }),
  import: (files: FileList | File[]) => {
    const body = new FormData();
    Array.from(files).forEach((file) => body.append("clips", file));
    return request<{ clips: Clip[] }>("/api/clips/import", { method: "POST", body });
  },
};
