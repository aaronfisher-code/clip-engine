# Setup

This guide is for people who are developing Clip Engine or operating the cloud
side. If you only want to install the desktop app, use the
[latest GitHub Release](https://github.com/aaronfisher-code/clip-engine/releases/latest)
and the [product README](../README.md).

Security boundaries, credentials, and incident notes live in
[`SECURITY.md`](SECURITY.md).

## Architecture

```text
Windows / Linux desktop
  egui UI → Rust engine → SQLite + FFmpeg/FFprobe
                         ├─ libmpv original-file playback
                         ├─ authenticated API → Cloudflare Worker → D1
                         └─ scoped multipart upload ──────────────→ R2

Public viewer → clips.dab.dev/c/<slug> → Worker share page
                                      → media.clips.dab.dev/published/... → R2
```

Public viewing is intentional. Creating uploads, listing the team library, extending
retention, approving users, and revoking users all require an active device token.
Anyone can request an account, but only owner approval creates a publishing account.
No email address or other personal information is collected. The desktop derives a
high-entropy credential from the password locally, so a member's human password never
leaves the machine.

The desktop application is Rust + egui + libmpv. It stores its library in a local
SQLite database and keeps account credentials in the operating-system credential
vault. The cloud control plane is a Cloudflare Worker backed by D1 and R2. Parent R2
credentials never enter the app: the Worker issues one-hour credentials restricted to
the exact video and thumbnail keys created for one upload.

## What is implemented

- Native Windows NSIS and Linux AppImage/deb installers.
- Native multi-file picker and an OBS inbox under the user's Videos directory.
- FFprobe metadata, original-file libmpv playback at source frame rate (including 120 fps),
  hardware decode when the GPU allows it, frame-accurate trimming, and audio-track mix
  without re-encoding video.
- Runtime detection of NVENC, Intel QSV, AMD AMF, with libx264 fallback.
- Local output at the display's native resolution and up to 240 fps when the
  capture path and encoder support it, plus direct multipart R2 upload.
- A separately packaged libobs replay helper with capability-driven display,
  audio-track, FPS, and global-hotkey configuration.
- Local SQLite library plus a shared cloud library for all approved members.
- Owner approval, password-reset, and member revocation controls.
- Public Open Graph clip pages with separate video and thumbnail assets.
- Exact 30-day application expiry plus an R2 lifecycle backstop.
- Non-destructive import of the old `data/clip-engine.json` library on first launch.

## Local development

Install Node.js 24, Rust 1.95, FFmpeg/FFprobe, and libmpv (plus headers) on the
development machine. Then run:

```bash
npm install
npm run dev:desktop
```

On Linux install `libmpv-dev` (and a working `ffmpeg` on `PATH`). On Windows set
`MPV_LIB_DIR` to a libmpv import-library directory if pkg-config is unavailable.

The recorder helper is built independently from the editor:

```bash
LIBOBS_PATH=/usr/lib cargo build -p clip-engine-recorder --features obs
CLIP_ENGINE_RECORDER=target/debug/clip-engine-recorder \
  cargo run -p clip-engine
```

The helper only loads OBS when the Recorder panel is opened or capture is
started. `cargo run -p clip-engine-recorder --features obs -- --probe` prints
the detected displays, capture backend, audio nodes, encoders, and supported
frame-rate range. The probe requires a matching libobs runtime; the pinned
bindings reject a different OBS API version rather than risking an allocator or
module-loader crash. Use the bundled runtime in releases or set
`CLIP_ENGINE_OBS_ROOT` to a directory containing matching `data/` and
`obs-plugins/` trees. The runtime must also provide its `obs-ffmpeg-mux`
executable; the desktop supervisor stages it beside the recorder helper when
needed. When launching the helper directly on Linux, also include that
runtime's `lib/` directory in `LD_LIBRARY_PATH`; the desktop supervisor sets
this path automatically for bundled helpers.

Distribution OBS packages can be newer than the pinned wrapper. For example,
OBS 32.2.x must not be substituted for the pinned 32.0.4 runtime: libobs can
abort during startup even when the API major version matches. If the panel
shows no screens or encoders, run the probe directly and use its diagnostic;
install or unpack a matching runtime instead of symlinking a newer
`libobs.so`.

The release probe can also be run through the repository smoke script:

```bash
CLIP_ENGINE_RECORDER=target/release/clip-engine-recorder \
CLIP_ENGINE_OBS_ROOT=resources/obs \
npm run smoke:recorder
```

On a real graphical session, `node scripts/smoke-recorder.mjs --full` starts a
one-second replay, waits for the finalized replay, and verifies the effective
encoder, container, codec, dimensions, frame rate, duration, and audio streams
with `ffprobe` when available. It is opt-in because headless CI cannot grant an
X11 display or a Wayland portal session.

### Recorder quality modes

The Recorder panel has two encoding modes:

- **Automatic** selects the best available hardware encoder in AV1, HEVC,
  H.264, then software order. It uses the selected display's native size and
  refresh rate (capped at 240 fps), prefers quality-based rate control, and
  falls back to CBR/bitrate when the encoder does not expose a quality
  property.
- **Advanced** exposes only the properties reported by the selected libobs
  encoder. This includes quality/CQP or CQVBR, target and maximum bitrate,
  keyframe interval, preset, tuning, multipass, profile, look-ahead,
  adaptive quantization, B-frames, B-frame references, split encode, GPU,
  rescaling, container, and native custom options. Unsupported fields are
  omitted and unsupported custom properties are reported as fallbacks.

MKV is the default replay container. It is safer than MP4 if the helper or
machine stops unexpectedly and keeps each configured audio route on its own
track with its configured name in stream metadata. MP4 is available in Advanced
mode when a downstream workflow requires it. The release runtime includes the
pinned libobs libraries and plugins; users
do not need a separate OBS Studio installation. NVENC, QSV, or AMF still
requires the corresponding vendor driver and hardware support.

### Recorder platform requirements

- Windows uses OBS monitor capture, WASAPI system/microphone capture, and the
  WASAPI process-loopback source. Application routes are discovered from
  visible windows and use an encoded selector that prefers the executable, so
  changing Spotify/Discord window titles does not break the route. The system
  route remains the normal WASAPI output mix, so application tracks can overlap
  it.
- Windows also enumerates active Core Audio `eRender` endpoints and exposes
  optional **Playback-device tracks**. Each route stores the opaque
  `IMMDevice::GetId()` value rather than a friendly name and passes that exact
  ID to OBS `wasapi_output_capture` with device timing enabled. Route apps to
  Voicemeeter buses or Windows per-app output devices, refresh the Recorder
  capabilities, and add only the endpoints you want on separate tracks. Disable
  **System audio** when avoiding overlap. These tracks capture endpoint mixes;
  they do not subtract processes automatically, and a recreated virtual device
  can leave a saved route unavailable until it is refreshed and added again.
- X11 uses OBS `xshm_input` and PulseAudio-compatible sources.
- Wayland uses OBS PipeWire plus xdg-desktop-portal for screen selection.
  The optional `linux-pipewire-audio` OBS plugin provides application capture
  on both Linux display backends: the helper discovers active
  PipeWire/PipeWire-Pulse streams and routes each selected executable or
  application name to its own track. Applications that are not currently
  playing audio can be entered manually and the plugin will reconnect when
  they appear. When the plugin is available, the Audio tab's **Exclude enabled
  application tracks from System audio** option configures a PipeWire
  exclusion source, keeping selected application streams out of the system
  track while retaining other output audio. Compositors that do not provide a
  global-hotkey protocol may reject global replay hotkeys; the Recorder panel
  reports that diagnostic and manual Save replay remains available.
- Successful and failed replay saves emit a desktop notification and a short
  system sound from the helper, so a focused game still gets feedback.
- Replay files are written as encoded MKV packets in the helper's staging
  directory, waited until stable, and moved into the library inbox. Raw
  1080p/1440p frames are not retained by the editor.
- The helper reports its RSS locally. Expect replay storage to scale roughly
  with `bitrate × replay seconds ÷ 8`, plus OBS/capture overhead; there is no
  artificial memory ceiling, so high-bitrate, long, or software-encoded
  profiles can exceed the 150–300 MB 1080p60 starting estimate.

### Tray and login startup

The desktop keeps a tray icon alive while the replay helper is running. Closing
the editor window hides it rather than stopping the helper; use the tray menu's
Quit action to shut down the application. The Recorder panel has a
**Launch Clip Engine at login** setting, enabled by default. The generated
startup entry passes `--background`, which starts the app hidden and starts the
saved replay configuration. A red dot on the tray icon indicates that the replay
buffer is currently running.

Windows uses the current user's Run registry entry. Linux uses an XDG Autostart
desktop entry under `~/.config/autostart` (or `$XDG_CONFIG_HOME/autostart`).
The tray implementation uses GTK/AppIndicator on Linux, so development and CI
builds need `libgtk-3-dev` and `libappindicator3-dev`. A desktop environment
must provide a StatusNotifier/AppIndicator host for the icon to be visible;
GNOME and some Wayland sessions require an AppIndicator/tray extension. The
replay helper and global hotkey may still be limited by the compositor.
Deb packages declare `libappindicator3-1` as the tray runtime dependency;
AppImages carry the linked runtime libraries.

When launched from an AppImage, the app registers the current `APPIMAGE` path
when that environment variable is available. Moving the AppImage later can
require disabling and re-enabling **Launch Clip Engine at login**.

To prepare a release runtime, provide a published archive URL and its SHA-256:

```bash
OBS_RUNTIME_URL=https://example.invalid/obs-runtime.tar.xz \
OBS_RUNTIME_SHA256=<64-hex-digest> \
node scripts/prepare-libobs-runtime.mjs
```

Release CI uses platform-specific `OBS_RUNTIME_URL_LINUX` /
`OBS_RUNTIME_SHA256_LINUX` and `OBS_RUNTIME_URL_WINDOWS` /
`OBS_RUNTIME_SHA256_WINDOWS` secrets (the legacy unsuffixed names remain a
fallback). The archive must contain the matching `libobs` libraries, OBS
plugins, encoders, and `data/` tree for that target. For the hardware encoder
matrix it must include `obs-ffmpeg.so`, `obs-nvenc.so`, and `obs-qsv11.so` on
Linux, or the corresponding `.dll` files on Windows. `obs-ffmpeg` contains
Linux VAAPI and Windows AMD AMF support; AMD does not have a separate encoder
plugin in the current OBS runtime. Linux archives may use the standard OBS
install layout (`share/obs/` plus `lib/obs-plugins/`) as well as the flattened
layout. The preparation step supplements a Linux archive missing `obs-nvenc.so`
or `obs-qsv11.so` from the host OBS plugin package when available, and fails
when any required encoder module is still missing. The host still provides the
GPU vendor runtime: NVIDIA's driver, Intel oneVPL/VAAPI runtime, or AMD
Mesa/libva driver. Linux archives intended to support per-application audio
should also include
`obs-plugins/linux-pipewire-audio.so` and its matching
`data/obs-plugins/linux-pipewire-audio/` locale tree. Without that plugin the
recorder still provides system and microphone tracks and reports why
application routes are unavailable.

Do not ship an unverified system runtime in the installer. Include the OBS
license/source-offer files in `resources/obs` and
`resources/THIRD_PARTY_NOTICES.md`.

Playback uses libmpv on the original recording: `hwdec=auto-safe`, `hr-seek=yes`,
coalesced timeline seeks, `aid=` for one selected track, and `lavfi-complex` amix
when several tracks are enabled. Video is not transcoded for preview. Thumbnails are
lazy JPEGs. Publishing still transcodes locally to 1080p/120.

Playback shortcuts are Space, Left/Right, Shift+Left/Right, and I/O.

Useful checks:

```bash
npm run check
npm test
```

`npm run build:desktop` copies FFmpeg and FFprobe from `PATH` and builds a release
binary. Official releases download static x86-64 FFmpeg builds in CI and package
Windows/Linux installers with cargo-packager so friends do not need to install FFmpeg
separately. libmpv is bundled with those installers. The Windows NSIS script is
vendored at `crates/clip-engine/packaging/windows/installer.nsi` so the installer can
offer an optional File Explorer **Create a clip** verb (mkv, mp4, mov, webm, avi, m4v)
without replacing the default video player. Silent in-app updates keep the user's last
choice; they do not add the verb unless it was already enabled.

## Production setup, step by step

### 1. Create Cloudflare resources

Install dependencies and authenticate Wrangler:

```bash
npm install
npx wrangler login
npx wrangler r2 bucket create clip-engine-media-prod
npx wrangler d1 create clip-engine-prod
```

Put the returned D1 ID and your Cloudflare account ID into
[`cloud/wrangler.jsonc`](../cloud/wrangler.jsonc). The config expects these domains:

- `api.clips.dab.dev` and `clips.dab.dev` route to the Worker.
- `media.clips.dab.dev` is the public custom domain for the R2 bucket.

Attach `media.clips.dab.dev` to the bucket in **R2 → bucket → Settings → Custom
Domains**. Wrangler creates the two Worker custom domains during deployment.

### 2. Create the least-privilege R2 parent token

In **Storage & databases → R2 → Overview → Manage API Tokens**, create an R2 Object
Read & Write API token restricted to `clip-engine-media-prod`. Record its Access Key ID
and Secret Access Key when Cloudflare displays them.

The Worker uses those server-only values to locally sign short-lived credentials scoped
to two generated keys. This avoids a Cloudflare control-plane request for every upload.
A desktop never receives a bucket-wide or permanent credential.

### 3. Configure secrets

Generate two independent random values, for example:

```bash
openssl rand -base64 48
openssl rand -base64 48
```

Set the Worker secrets; do not put them in `wrangler.jsonc` or Git:

```bash
cd cloud
npx wrangler secret put BOOTSTRAP_TOKEN
npx wrangler secret put TOKEN_PEPPER
npx wrangler secret put R2_PARENT_SECRET_ACCESS_KEY
cd ..
```

Set `R2_PARENT_ACCESS_KEY_ID` in `cloud/wrangler.jsonc` to the Access Key ID from that
same R2 token. `R2_PARENT_SECRET_ACCESS_KEY` and `R2_PARENT_ACCESS_KEY_ID` must therefore
be the secret/ID pair created together. Keep the bootstrap token in an encrypted
password manager. It is the `admin` account's initial and recovery password. The
expected local-development secret names are also shown in
[`cloud/.dev.vars.example`](../cloud/.dev.vars.example).

### 4. Apply the schema and lifecycle policy

```bash
npm --workspace @clip-engine/cloud exec -- wrangler d1 migrations apply clip-engine-prod --remote
npm --workspace @clip-engine/cloud exec -- wrangler r2 bucket lifecycle set clip-engine-media-prod --file lifecycle.json
```

Run the lifecycle command from the `cloud/` directory, or use `--file
cloud/lifecycle.json` from the repository root. The lifecycle deletes objects beneath
`published/` 30 days after their most recent write and aborts unfinished multipart
uploads after one day. The Worker independently stops serving a link at its exact D1
expiry; R2 physical deletion can follow within roughly 24 hours.

### 5. Deploy and verify the Worker

```bash
npm --workspace @clip-engine/cloud exec -- wrangler deploy
curl https://api.clips.dab.dev/health
```

The repository also contains a manually triggered **Cloud deployment** GitHub Actions
workflow. Add `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` as repository secrets
before using it.

### 6. Sign in as the owner and approve members

Launch a development build or installer, choose **Sign in**, and use `admin` as the
username and the `BOOTSTRAP_TOKEN` value as the password. On first sign-in this creates
the owner account; the same login also recovers or migrates an existing owner account.
Afterward the desktop keeps the owner signed in in the same way as every other user.

Normal sign-in needs only the username and password. The resulting random device token
is stored in Windows Credential Manager or the Linux Secret Service keyring, while D1
stores only a peppered SHA-256 hash. This keeps the user signed in across restarts and
application updates. Signing out revokes only that device server-side before removing
the local credential.

On a friend's first launch, **Create account** asks only for a username, display name,
and password. **Request access** places the account in the owner's review queue and
saves a status token in the operating-system credential vault. The waiting screen tells
them that the owner will notify them; they can use **Check status** after you do.

Open **Manage access → Pending** to filter, approve, or decline requests. Approval
creates a durable member account. Tell the member they can now sign in; they remain
signed in until they explicitly sign out or you revoke them. Restoring a revoked
account does not reactivate old device sessions.

If a member forgets their password, open **Manage access → Active**, choose **Reset
password**, and send them the generated one-day token or link privately. They choose
**Sign in → Forgot my password**, enter their username and that token, and can select a
new password only after the Worker validates both. Redeeming it signs them in and
revokes their older device sessions.

PBKDF2-HMAC-SHA256 with 600,000 iterations runs asynchronously on the desktop. The
Worker performs only a peppered SHA-256 verification of that derived credential, keeping
routine authentication beneath the Workers Free CPU ceiling. Successful account
requests are capped at five per source IP per day, and waiting clients check status only
when asked, limiting idle Worker and D1 usage. For a small private group this should fit
comfortably inside the free control-plane allowances. R2 storage and operation usage
still depends on how many gigabytes of clips the group publishes during each 30-day
window.

### 7. Publish desktop installers

The `release` branch is the automatic publisher. Every push to it patch-bumps the
desktop version, builds Windows NSIS and Linux AppImage/deb packages with bundled
FFmpeg and libmpv, then **publishes** a GitHub Release. Friends on a build that
includes the in-app updater are offered that release after the workflow finishes.

Create the branch once from the commit you want to ship:

```bash
git checkout rust-rewrite
git checkout -b release
git push -u origin release
```

Later, merge or cherry-pick onto `release` and push:

```bash
git checkout release
git merge rust-rewrite
git push
```

Put `[minor]` or `[major]` in the commit message to bump those instead of patch.
The workflow writes a `chore(release): vX.Y.Z` commit and tag after both
installers succeed, so a failed build does not consume a version number.

To ship a **draft** for testing without publishing, push a version tag from any
branch (after bumping `package.json` and `Cargo.toml` yourself):

```bash
git tag v1.0.1
git push origin v1.0.1
```

The desktop checks
`https://api.github.com/repos/aaronfisher-code/clip-engine/releases/latest`
on launch (and when **Check for updates** is used). Drafts and pre-releases are
ignored. Windows installs the new NSIS package silently for the current user;
Linux prefers the AppImage (replacing the running AppImage when `APPIMAGE` is set)
and otherwise opens the downloaded package. There is no extra update server:
the installer files already attached to the GitHub Release are the update payload.

Windows Authenticode signing can be added before broad distribution if you want to
avoid SmartScreen reputation warnings. Linux package signing can likewise be added for
an apt repository. Auto-update itself verifies the download over HTTPS from GitHub;
it does not yet verify a separate packager signature.

## Storage and deletion behavior

Removing a local clip permanently deletes its original recording from the device,
along with its preview, exports, and SQLite history. Deleting a published version
deletes its R2 video and thumbnail plus the local export. Extending a clip rewrites
its two R2 objects and advances its D1 expiry by another 30 days.

Desktop data is stored in the platform application-data directory:

- Windows: `%LOCALAPPDATA%\dev.dab.clip-engine`
- Linux: `$XDG_DATA_HOME/dev.dab.clip-engine` or `~/.local/share/dev.dab.clip-engine`

The SQLite database is the only irreplaceable local app file. Published versions are
managed separately from local originals. The cloud data plane is small enough that Workers, D1, and R2
are normally simpler and cheaper than exposing a home server; the home server is better
used for encrypted backups of the repository, signing key, and optional source clips.

## Production checklist

- Replace every `replace-with-*` value in `cloud/wrangler.jsonc`.
- Verify all three custom domains and HTTPS before distributing installers.
- Apply `cloud/lifecycle.json` and confirm it with `wrangler r2 bucket lifecycle list`.
- Store Worker and GitHub secrets only in their respective secret stores.
- Test owner sign-in, account approval, password reset, revocation, multipart upload, expiry, and
  a desktop installer using a staging account first.
- Enable GitHub branch protection and dependency/security update automation.
