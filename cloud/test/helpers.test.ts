import { describe, expect, it } from "vitest";
import { cleanCredential, cleanUsername, validateUploadIntent } from "../src";
import { cors } from "../src/http";
import { escapeHtml, sharePage } from "../src/share";
import { createR2TemporaryCredentials } from "../src/r2-credentials";

describe("cloud input boundaries", () => {
  it("escapes user-controlled share titles", () => {
    expect(escapeHtml(`<script>alert("x")</script>`)).toBe("&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;");
  });

  it("renders the branded viewer and Discord video metadata with its uploader", () => {
    const page = sharePage({
      title: 'Ace & <escape> "this"',
      siteName: "Dabs Clip Engine",
      uploaderName: "DAB & friends",
      pageUrl: "https://clips.example/c/ace",
      mediaUrl: "https://media.example/published/ace/video.mp4",
      thumbnailUrl: "https://media.example/published/ace/thumbnail.jpg",
      duration: 65.4,
      width: 1920,
      height: 1080,
      fps: 120,
      publishedAt: "2026-08-14T00:00:00.000Z",
      expiresAt: "2026-09-13T00:00:00.000Z",
    });
    expect(page).toContain('property="og:site_name" content="Dabs Clip Engine"');
    expect(page).toContain('property="og:title" content="Ace &amp; &lt;escape&gt; &quot;this&quot;"');
    expect(page).toContain('property="og:video:secure_url" content="https://media.example/published/ace/video.mp4"');
    expect(page).toContain('content="1:05 · 1920×1080 · 120 FPS · by DAB &amp; friends"');
    expect(page).toContain('<span class="eyebrow">Published by DAB &amp; friends</span>');
    expect(page).toContain('<video controls playsinline preload="metadata"');
    expect(page).not.toContain("<escape>");
  });

  it("accepts a valid upload intent", () => {
    expect(validateUploadIntent({
      title: "  Round   win  ", videoSize: 100, thumbnailSize: 10, duration: 4,
      width: 1920, height: 1080, fps: 120,
    }, 1_000).title).toBe("Round win");
  });

  it("rejects an upload larger than the configured maximum", () => {
    expect(() => validateUploadIntent({
      title: "Too big", videoSize: 1_001, thumbnailSize: 10, duration: 4,
      width: 1920, height: 1080, fps: 120,
    }, 1_000)).toThrow(/Video size/);
  });

  it("allows only desktop development origins through CORS", () => {
    const trusted = cors(new Request("https://api.example.test", { headers: { origin: "tauri://localhost" } }), new Response());
    const untrusted = cors(new Request("https://api.example.test", { headers: { origin: "https://evil.example" } }), new Response());
    expect(trusted.headers.get("access-control-allow-origin")).toBe("tauri://localhost");
    expect(untrusted.headers.get("access-control-allow-origin")).toBeNull();
    expect(trusted.headers.get("x-content-type-options")).toBe("nosniff");
  });

  it("normalizes usernames and rejects unsafe forms", () => {
    expect(cleanUsername("  Aaron-Clips ")).toBe("aaron-clips");
    expect(() => cleanUsername("no spaces")).toThrow(/Username/);
    expect(() => cleanUsername("ab")).toThrow(/Username/);
  });

  it("accepts only a 256-bit URL-safe derived credential", () => {
    expect(cleanCredential("A".repeat(43))).toHaveLength(43);
    expect(() => cleanCredential("human password 12345")).toThrow(/credential/);
  });

  it("locally signs path-scoped R2 temporary credentials", async () => {
    const credentials = await createR2TemporaryCredentials({
      accountId: "a".repeat(32),
      accessKeyId: "b".repeat(32),
      secretAccessKey: "parent-secret",
      bucket: "clips",
      objects: ["published/clip/video.mp4", "published/clip/thumbnail.jpg"],
      ttlSeconds: 3_600,
      nowSeconds: 1_700_000_000,
    });
    const jwt = atob(credentials.sessionToken).replace(/^jwt\//, "");
    const payloadPart = jwt.split(".")[1].replaceAll("-", "+").replaceAll("_", "/");
    const payload = JSON.parse(atob(payloadPart));
    expect(payload).toMatchObject({
      bucket: "clips",
      scope: "object-read-write",
      sub: "a".repeat(32),
      iss: "b".repeat(32),
      iat: 1_700_000_000,
      exp: 1_700_003_600,
      paths: { prefixPaths: [], objectPaths: ["published/clip/video.mp4", "published/clip/thumbnail.jpg"] },
    });
    expect(credentials.accessKeyId).toBe("b".repeat(32));
    expect(credentials.secretAccessKey).toMatch(/^[a-f0-9]{64}$/);
    expect(jwt.split(".")).toHaveLength(3);
  });

});
