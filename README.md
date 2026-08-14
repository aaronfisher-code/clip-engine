# Clip Engine

A local-first clipping workstation for high-frame-rate game recordings. Clip Engine imports native recordings (including multi-track MKV files), makes a lightweight browser preview, lets you set in/out points and choose an audio mix, then exports a 1080p120 MP4 and uploads it to Cloudflare R2.

The browser never receives R2 credentials and the service only listens on `127.0.0.1`.

## What works

- Scan an OBS replay-buffer folder, use the file picker, or drag recordings anywhere onto the UI.
- Probe resolution, frame rate, codec, duration, and every audio stream with FFprobe.
- Preview MKV and other browser-incompatible sources through an automatic 720p30 proxy.
- Trim at source-frame increments and select any combination of audio tracks.
- Mix the selected tracks into one AAC stream so browsers and Discord play the intended mix.
- Transcode to a fast-start 1920×1080, 120 fps H.264 MP4.
- Multipart-upload large exports to R2 with live progress and copy the custom-domain URL.
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

Get the account ID and API token from **Cloudflare Dashboard → R2 Object Storage → Manage R2 API Tokens**. The public base URL is the custom domain connected to the bucket; Clip Engine uploads objects under `YYYY-MM-DD/clip-name-id.mp4`, so the result is a direct video URL such as `https://clips.dab.dev/2026-08-14/round-win-a1b2c3d4.mp4`.

For a production-style local run:

```bash
npm run build
npm start
```

Then open <http://127.0.0.1:4317>.

## Recording with OBS

Replay Buffer is a good capture front end: OBS already handles hotkeys, GPU capture, high-frame-rate encoding, and separate application/microphone tracks reliably.

1. In **Settings → Video**, set the output resolution to `2560x1440` and FPS to `120`.
2. In **Settings → Output**, use Advanced mode, enable Replay Buffer, and record to MKV. MKV is resilient to crashes and supports multiple audio tracks.
3. Assign desktop/game and microphone sources to separate tracks in **Advanced Audio Properties**.
4. Set the recording directory to the absolute `CLIP_SOURCE_DIR` from `.env` (the default is `./data/inbox`).
5. Start Replay Buffer in OBS and use its **Save Replay** hotkey. Press the refresh button in Clip Engine to discover saved files.

The exact recording encoder and bitrate depend on the GPU. Recording to a high-quality hardware codec preserves quality without competing heavily with the game; the final shared file remains H.264 for dependable inline playback.

## Export performance

The default `libx264` encoder maximizes compatibility but 1080p120 software encoding can be demanding. NVIDIA users can enable hardware export:

```dotenv
FFMPEG_VIDEO_ENCODER=h264_nvenc
FFMPEG_PRESET=p5
FFMPEG_CRF=20
```

For NVENC, `FFMPEG_CRF` is used as the constant-quality (`CQ`) value. Intel QSV (`h264_qsv`) and AMD AMF (`h264_amf`) are also recognized when supported by the local FFmpeg build. Hardware output quality and available encoders vary by driver.

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
