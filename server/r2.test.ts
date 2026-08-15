import assert from "node:assert/strict";
import test from "node:test";
import { publicR2Key } from "./r2.js";

test("extracts an R2 key only from the configured public URL", () => {
  const base = "https://clips.example.com/public";
  assert.equal(
    publicR2Key("https://clips.example.com/public/media/2026-08-15/my%20clip.mp4", base),
    "media/2026-08-15/my clip.mp4",
  );
  assert.equal(publicR2Key("https://other.example.com/public/media/clip.mp4", base), undefined);
  assert.equal(publicR2Key("https://clips.example.com/publicity/media/clip.mp4", base), undefined);
  assert.equal(publicR2Key("not a URL", base), undefined);
});
