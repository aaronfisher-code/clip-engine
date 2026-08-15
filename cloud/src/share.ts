export type SharePageDetails = {
  title: string;
  siteName: string;
  uploaderName: string;
  pageUrl: string;
  mediaUrl: string;
  thumbnailUrl: string;
  duration: number;
  width: number;
  height: number;
  fps: number;
  publishedAt: string;
  expiresAt: string;
};

export function escapeHtml(value: string) {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;",
  })[character] || character);
}

function formatDuration(seconds: number) {
  const rounded = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(rounded / 60);
  return `${minutes}:${(rounded % 60).toString().padStart(2, "0")}`;
}

function formatDate(value: string) {
  return new Date(value).toLocaleDateString("en-AU", {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  });
}

export function sharePage(clip: SharePageDetails) {
  const title = escapeHtml(clip.title);
  const siteName = escapeHtml(clip.siteName);
  const uploaderName = escapeHtml(clip.uploaderName);
  const pageUrl = escapeHtml(clip.pageUrl);
  const mediaUrl = escapeHtml(clip.mediaUrl);
  const thumbnailUrl = escapeHtml(clip.thumbnailUrl);
  const duration = formatDuration(clip.duration);
  const technicalDetails = `${duration} · ${clip.width}×${clip.height} · ${Math.round(clip.fps)} FPS`;
  const description = escapeHtml(`${technicalDetails} · by ${clip.uploaderName}`);
  const published = escapeHtml(formatDate(clip.publishedAt));
  const expires = escapeHtml(formatDate(clip.expiresAt));

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
  <title>${title} · ${siteName}</title>
  <meta name="description" content="${description}">
  <meta name="author" content="${uploaderName}">
  <meta name="theme-color" content="#c7ff3d">
  <link rel="canonical" href="${pageUrl}">

  <meta property="og:type" content="video.other">
  <meta property="og:site_name" content="${siteName}">
  <meta property="og:title" content="${title}">
  <meta property="og:description" content="${description}">
  <meta property="og:url" content="${pageUrl}">
  <meta property="og:image" content="${thumbnailUrl}">
  <meta property="og:image:secure_url" content="${thumbnailUrl}">
  <meta property="og:image:type" content="image/jpeg">
  <meta property="og:image:width" content="1280">
  <meta property="og:image:height" content="720">
  <meta property="og:image:alt" content="Preview frame from ${title}">
  <meta property="og:video" content="${mediaUrl}">
  <meta property="og:video:url" content="${mediaUrl}">
  <meta property="og:video:secure_url" content="${mediaUrl}">
  <meta property="og:video:type" content="video/mp4">
  <meta property="og:video:width" content="${clip.width}">
  <meta property="og:video:height" content="${clip.height}">
  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="${title}">
  <meta name="twitter:description" content="${description}">
  <meta name="twitter:image" content="${thumbnailUrl}">

  <style>
    :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #08090b; color: #f4f5f7; }
    * { box-sizing: border-box; }
    body { min-height: 100vh; margin: 0; background: radial-gradient(circle at 50% -20%, #283016 0, #0e1011 34rem, #08090b 70rem); }
    button, a { font: inherit; }
    .shell { width: min(1120px, calc(100% - 32px)); margin: 0 auto; padding: 32px 0 64px; }
    header { display: flex; align-items: center; justify-content: space-between; gap: 24px; margin-bottom: 28px; }
    .brand { display: flex; align-items: center; gap: 11px; color: #f4f5f7; font-weight: 750; letter-spacing: -.02em; }
    .brand-mark { width: 13px; height: 13px; border-radius: 3px; background: #c7ff3d; box-shadow: 0 0 24px #c7ff3d80; transform: rotate(45deg); }
    .source { color: #7c818b; font-size: 13px; text-decoration: none; }
    .source:hover { color: #c7ff3d; }
    main { border: 1px solid #292d32; border-radius: 18px; background: #111315; overflow: hidden; box-shadow: 0 28px 90px #0009; }
    .player { position: relative; aspect-ratio: 16 / 9; background: #000; }
    video { display: block; width: 100%; height: 100%; object-fit: contain; }
    .details { display: flex; align-items: end; justify-content: space-between; gap: 28px; padding: 24px 26px 26px; border-top: 1px solid #24272c; }
    .eyebrow { display: block; margin-bottom: 9px; color: #c7ff3d; font: 700 11px/1.2 ui-monospace, SFMono-Regular, Menlo, monospace; letter-spacing: .13em; text-transform: uppercase; }
    h1 { max-width: 800px; margin: 0 0 12px; overflow-wrap: anywhere; font-size: clamp(22px, 4vw, 36px); line-height: 1.08; letter-spacing: -.035em; }
    .meta { display: flex; flex-wrap: wrap; gap: 8px; color: #9297a1; font-size: 13px; }
    .meta span { padding: 6px 9px; border: 1px solid #2b2f34; border-radius: 6px; background: #17191c; }
    .author { color: #dce0e4 !important; border-color: #48552b !important; background: #18200f !important; }
    .actions { display: flex; flex-shrink: 0; gap: 9px; }
    .actions a, .actions button { min-height: 40px; padding: 0 15px; border: 1px solid #353a40; border-radius: 8px; background: #191c1f; color: #e7e9eb; text-decoration: none; cursor: pointer; }
    .actions a { display: inline-flex; align-items: center; }
    .actions button { border-color: #a7d72f; background: #c7ff3d; color: #111408; font-weight: 750; }
    .actions a:hover { border-color: #666d76; }
    .actions button:hover { background: #d4ff69; }
    footer { padding: 19px 3px 0; color: #646972; font-size: 12px; text-align: center; }
    @media (max-width: 700px) {
      .shell { width: min(100% - 20px, 1120px); padding-top: 18px; }
      header { margin: 0 5px 18px; }
      main { border-radius: 12px; }
      .details { align-items: stretch; flex-direction: column; padding: 20px; }
      .actions { display: grid; grid-template-columns: 1fr 1fr; }
      .actions a, .actions button { justify-content: center; text-align: center; }
    }
  </style>
</head>
<body>
  <div class="shell">
    <header>
      <div class="brand"><span class="brand-mark"></span>${siteName}</div>
      <a class="source" href="${mediaUrl}">Direct video ↗</a>
    </header>
    <main>
      <div class="player">
        <video controls playsinline preload="metadata" poster="${thumbnailUrl}" src="${mediaUrl}"></video>
      </div>
      <section class="details">
        <div>
          <span class="eyebrow">Published by ${uploaderName}</span>
          <h1>${title}</h1>
          <div class="meta"><span>${duration}</span><span>${clip.width}×${clip.height}</span><span>${Math.round(clip.fps)} FPS</span><span class="author">${uploaderName}</span><span>${published}</span><span>Expires ${expires}</span></div>
        </div>
        <div class="actions">
          <a href="${mediaUrl}" download>Download</a>
          <button id="share" type="button">Share clip</button>
        </div>
      </section>
    </main>
    <footer>Hosted with ${siteName} · uploaded by ${uploaderName}</footer>
  </div>
  <script>
    document.getElementById("share").addEventListener("click", async function () {
      try {
        if (navigator.share) await navigator.share({ title: document.title, url: location.href });
        else { await navigator.clipboard.writeText(location.href); this.textContent = "Link copied"; }
      } catch (error) {
        if (error && error.name !== "AbortError") this.textContent = "Copy failed";
      }
    });
  </script>
</body>
</html>`;
}

export function gonePage() {
  return `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="theme-color" content="#c7ff3d"><title>Clip expired · Dabs Clip Engine</title><style>:root{color-scheme:dark;font-family:Inter,ui-sans-serif,system-ui;background:#08090b;color:#f4f5f7}body{margin:0;display:grid;place-items:center;min-height:100vh;background:radial-gradient(circle at 50% -20%,#283016 0,#0e1011 34rem,#08090b 70rem);text-align:center}main{padding:42px;border:1px solid #292d32;border-radius:18px;background:#111315;box-shadow:0 28px 90px #0009}.mark{width:16px;height:16px;margin:0 auto 22px;border-radius:4px;background:#c7ff3d;box-shadow:0 0 24px #c7ff3d80;transform:rotate(45deg)}h1{margin:0 0 10px;font-size:26px}p{margin:0;color:#9297a1}</style><main><div class="mark"></div><h1>This clip has expired.</h1><p>Published clips are kept for 30 days.</p></main></html>`;
}
