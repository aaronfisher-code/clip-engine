# Clip Engine

A local-first clipping workstation for high-frame-rate game recordings. Clip Engine imports native recordings (including multi-track MKV files), makes a lightweight browser preview, lets you set in/out points and choose an audio mix, then exports a 1080p120 MP4 and uploads it to Cloudflare R2.

The browser never receives R2 credentials and the service only listens on `127.0.0.1`.

## What works

- Scan an OBS replay-buffer folder, use the file picker, or drag recordings anywhere onto the UI.
- Probe resolution, frame rate, codec, duration, and every audio stream with FFprobe.
- Preview MKV and other browser-incompatible sources through an automatic 720p30 proxy.
- Trim at source-frame increments and select any combination of audio tracks.
- Mix the selected tracks into one AAC stream so browsers and Discord play the intended mix.
- Transcode to a fast-start 1920×1080, 120 fps H.264 MP4 with stream-friendly NVENC rate control.
- Multipart-upload large exports to R2 with live progress and copy a branded share-page URL.
- Generate a 1280×720 thumbnail and Open Graph metadata for titled, playable Discord embeds.
- Serve every published clip on a responsive public page with playback, sharing, and download controls.
- Retain the original recording unchanged and persist the local library/job history.

## Prerequisites

- Node.js 22 or newer
- FFmpeg and FFprobe on `PATH`
- An R2 bucket with a custom public domain (for example `clips.dab.dev`)
- An R2 API token with Object Read & Write permission for that bucket

## Setup

```bash
npm install
cp .env.example .env
npm run dev
```

Open <http://127.0.0.1:4318> during development. Fill these values in `.env` before publishing:

```dotenv
R2_ACCOUNT_ID=your-cloudflare-account-id
R2_ACCESS_KEY_ID=your-r2-token-access-key
R2_SECRET_ACCESS_KEY=your-r2-token-secret
R2_BUCKET=clips
R2_PUBLIC_BASE_URL=https://clips.dab.dev
```

If OBS stores generic names such as `Track1`, give the audio streams useful fallback labels in recording-track order:

```dotenv
CLIP_AUDIO_TRACK_LABELS=Game / System,Discord,Microphone
```

Embedded non-generic track titles still take precedence, so recordings from other sources keep their own labels.

Set the brand name used by public clip pages and Discord embeds if you do not want the default:

```dotenv
CLIP_SHARE_SITE_NAME=DAB Clips
```

Get the account ID and API token from **Cloudflare Dashboard → R2 Object Storage → Manage R2 API Tokens**. The public base URL must be a custom domain connected to the bucket. Each publish creates three immutable objects: the MP4 under `media/YYYY-MM-DD/`, a JPEG under `thumbnails/YYYY-MM-DD/`, and an extensionless HTML share page under `clips/`. Clip Engine copies the share-page URL, such as `https://clips.dab.dev/clips/round-win-a1b2c3d4`; the page advertises the direct MP4 to Discord as playable Open Graph video media.

`CLIP_SHARE_SITE_NAME` controls the small brand name shown on the public page and in its embed metadata. Discord caches link previews, so changes affect new publishes immediately but an already-posted URL may retain its original preview for a while.

Only newly published clips receive share pages and thumbnails. Existing R2 video URLs stay valid; publish a clip again if you want a new Discord-ready share link for it.

For a production-style local run:

```bash
npm run build
npm start
```

Then open <http://127.0.0.1:4317>.

## Run continuously as a Linux service

Clip Engine includes a systemd user-service installer. It builds the production app, starts it in the background immediately, enables it for future logins, and restarts it if the process crashes:

```bash
npm run service:install
```

Bookmark <http://127.0.0.1:4317> and open it whenever you want to manage clips. A terminal does not need to stay open.

Useful service commands:

```bash
npm run service:status    # Current state and recent output
npm run service:logs      # Follow server and FFmpeg logs
npm run service:restart   # Reload code or changes made to .env
npm run service:remove    # Stop and remove only the service
```

The service normally starts when you log into the desktop. To start it during boot and keep it running after logout, enable systemd user lingering once:

```bash
loginctl enable-linger "$USER"
```

After updating Clip Engine, rebuild and restart the service:

```bash
npm install
npm run build
npm run service:restart
```

Changes to `.env` always require `npm run service:restart`. The installer stores the generated unit at `~/.config/systemd/user/clip-engine.service`; removing the service keeps `.env`, original recordings, previews, exports, and library history intact.

## Recording with OBS

Replay Buffer is a good capture front end: OBS already handles hotkeys, GPU capture, high-frame-rate encoding, and separate application/microphone tracks reliably.

1. In **Settings → Video**, set the output resolution to `2560x1440` and FPS to `120`.
2. In **Settings → Output**, use Advanced mode, enable Replay Buffer, and record to MKV. MKV is resilient to crashes and supports multiple audio tracks.
3. Assign desktop/game and microphone sources to separate tracks in **Advanced Audio Properties**.
4. Set the recording directory to the absolute `CLIP_SOURCE_DIR` from `.env` (the default is `./data/inbox`).
5. Start Replay Buffer in OBS and use its **Save Replay** hotkey. Press the refresh button in Clip Engine to discover saved files.

The exact recording encoder and bitrate depend on the GPU. Recording to a high-quality hardware codec preserves quality without competing heavily with the game; the final shared file remains H.264 for dependable inline playback.

## Export quality and performance

The default `libx264` encoder uses its `slow` preset, CRF 18, and a 30 Mbps ceiling. This retains broadly compatible 1080p120 H.264 playback while spending CPU time to get better quality from the available bitrate:

```dotenv
FFMPEG_VIDEO_ENCODER=libx264
FFMPEG_PRESET=slow
FFMPEG_CRF=18
```

NVIDIA users can trade some compression efficiency for faster hardware exports:

```dotenv
FFMPEG_VIDEO_ENCODER=h264_nvenc
FFMPEG_PRESET=p6
FFMPEG_CRF=21
```

For NVENC, `FFMPEG_CRF` is used as the constant-quality (`CQ`) target. Clip Engine combines it with high-quality multipass encoding, adaptive quantization, a 20 Mbps target, and a 30 Mbps ceiling. Intel QSV (`h264_qsv`) and AMD AMF (`h264_amf`) are also recognized when supported by the local FFmpeg build. Hardware output quality and available encoders vary by driver.

## Local data

By default, runtime files live under `data/` and are ignored by Git:

```text
data/
├── inbox/       # watched OBS directory
├── sources/     # files imported through the browser
├── previews/    # disposable 720p30 proxies
├── exports/     # completed 1080p120 exports
└── clip-engine.json
```

Clip Engine intentionally keeps local exports after upload. Originals and exports are never automatically deleted.

## Commands

```bash
npm run check    # TypeScript checks for server and UI
npm test         # FFmpeg argument/unit tests
npm run build    # Server compile and production UI bundle
```

## Current scope

This first version uses OBS or another recorder for capture rather than replacing the capture stack. It scans only the top level of `CLIP_SOURCE_DIR`, processes one preview at a time, and runs publish jobs in-process. Restarted jobs are marked failed and can be submitted again. R2 credentials stay server-side in `.env`; do not commit that file.
