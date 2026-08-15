import assert from "node:assert/strict";
import test from "node:test";
import { createSharePage } from "./share-page.js";

const page = createSharePage({
  title: 'Ace & <escape> "this"',
  siteName: "DAB Clips",
  pageUrl: "https://clips.example/clips/ace",
  videoUrl: "https://clips.example/media/ace.mp4",
  thumbnailUrl: "https://clips.example/thumbnails/ace.jpg",
  duration: 65.4,
  width: 1920,
  height: 1080,
  fps: 120,
  publishedAt: "2026-08-14T00:00:00.000Z",
});

test("creates Discord-friendly image and playable video metadata", () => {
  assert.match(page, /property="og:title" content="Ace &amp; &lt;escape&gt; &quot;this&quot;"/);
  assert.match(page, /property="og:image" content="https:\/\/clips\.example\/thumbnails\/ace\.jpg"/);
  assert.match(page, /property="og:video:secure_url" content="https:\/\/clips\.example\/media\/ace\.mp4"/);
  assert.match(page, /property="og:video:type" content="video\/mp4"/);
  assert.match(page, /name="theme-color" content="#c7ff3d"/);
});

test("renders a responsive browser player with useful clip details", () => {
  assert.match(page, /<video controls playsinline[^>]+poster="https:\/\/clips\.example\/thumbnails\/ace\.jpg"[^>]+src="https:\/\/clips\.example\/media\/ace\.mp4"/);
  assert.match(page, /1:05 · 1920×1080 · 120 FPS/);
  assert.match(page, /<h1>Ace &amp; &lt;escape&gt; &quot;this&quot;<\/h1>/);
  assert.doesNotMatch(page, /<h1>.*<escape>/);
});
