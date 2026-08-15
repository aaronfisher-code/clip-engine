export type SharePageDetails = {
  title: string;
  siteName: string;
  pageUrl: string;
  videoUrl: string;
  thumbnailUrl: string;
  duration: number;
  width: number;
  height: number;
  fps: number;
  publishedAt: string;
};

function escapeHtml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function formatDuration(seconds: number) {
  const rounded = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(rounded / 60);
  const remainder = rounded % 60;
  return `${minutes}:${remainder.toString().padStart(2, "0")}`;
}

export function createSharePage(details: SharePageDetails) {
  const title = escapeHtml(details.title);
  const siteName = escapeHtml(details.siteName);
  const pageUrl = escapeHtml(details.pageUrl);
  const videoUrl = escapeHtml(details.videoUrl);
  const thumbnailUrl = escapeHtml(details.thumbnailUrl);
  const duration = formatDuration(details.duration);
  const description = escapeHtml(`${duration} · ${details.width}×${details.height} · ${Math.round(details.fps)} FPS`);
  const published = escapeHtml(new Date(details.publishedAt).toLocaleDateString("en-AU", {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  }));

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
  <title>${title} · ${siteName}</title>
  <meta name="description" content="${description}">
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
  <meta property="og:video" content="${videoUrl}">
  <meta property="og:video:url" content="${videoUrl}">
  <meta property="og:video:secure_url" content="${videoUrl}">
  <meta property="og:video:type" content="video/mp4">
  <meta property="og:video:width" content="${details.width}">
  <meta property="og:video:height" content="${details.height}">
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
      <a class="source" href="${videoUrl}">Direct video ↗</a>
    </header>
    <main>
      <div class="player">
        <video controls playsinline preload="metadata" poster="${thumbnailUrl}" src="${videoUrl}"></video>
      </div>
      <section class="details">
        <div>
          <span class="eyebrow">Published clip</span>
          <h1>${title}</h1>
          <div class="meta"><span>${duration}</span><span>${details.width}×${details.height}</span><span>${Math.round(details.fps)} FPS</span><span>${published}</span></div>
        </div>
        <div class="actions">
          <a href="${videoUrl}" download>Download</a>
          <button id="share" type="button">Share clip</button>
        </div>
      </section>
    </main>
    <footer>Hosted with ${siteName}</footer>
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
