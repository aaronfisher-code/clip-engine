# Clip Engine

Clip Engine is a local-first Windows and Linux desktop app for trimming recordings,
mixing audio tracks, transcoding with the local GPU or CPU, and publishing public
30-day clip links. Publishing requires owner approval and can be revoked at any time.

The desktop application is Tauri 2 + React + Rust. It stores its library in a local
SQLite database and keeps account credentials in the operating-system credential
vault. The cloud control plane is a Cloudflare Worker backed by D1 and R2. Parent R2
credentials never enter the app: the Worker issues one-hour credentials restricted to
the exact video and thumbnail keys created for one upload.

## Architecture

```text
Windows / Linux desktop
  React UI → Tauri commands → SQLite + FFmpeg/FFprobe
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

## What is implemented

- Native Windows NSIS and Linux AppImage/deb installers.
- Signed automatic updates from GitHub Releases.
- Native multi-file picker and an OBS inbox under the user's Videos directory.
- FFprobe metadata, instant original-file playback, LosslessCut-style coalesced seeking,
  FFmpeg-assisted fallback for incompatible codecs, frame-aware trimming, and
  audio-track mix.
- Runtime detection of NVENC, Intel QSV, AMD AMF, with libx264 fallback.
- Local 1080p output up to 120 fps and direct multipart R2 upload.
- Local SQLite library plus a shared cloud library for all approved members.
- Owner approval, password-reset, and member revocation controls.
- Public Open Graph clip pages with separate video and thumbnail assets.
- Exact 30-day application expiry plus an R2 lifecycle backstop.
- Non-destructive import of the old `data/clip-engine.json` library on first launch.

## Local development

Install Node.js 24, Rust 1.93, Tauri's platform prerequisites, and FFmpeg/FFprobe on
`PATH`. Then run:

```bash
npm install
npm run dev:desktop
```

On NVIDIA Wayland sessions the app automatically disables WebKitGTK's problematic
DMA-BUF renderer before creating the window. This avoids the upstream `Gdk Error 71`
startup crash without changing FFmpeg GPU transcoding.

Playback follows the same broad strategy as LosslessCut: use the original file directly
when the platform decoder supports it and avoid an up-front transcode. Rapid timeline
drags are coalesced so only the newest seek is decoded, without pausing playback. If the
native webview genuinely cannot decode a source, the app seeks FFmpeg to the requested
position and streams a full-resolution, source-frame-rate, all-intra fragmented MP4 over
a random, tokenized loopback URL. A paused scrub decodes one display frame and then
closes the stream; continuous transcoding happens only while playback is running. The
same fallback is available on Windows and Linux, but it is never selected merely because
of a filename or codec heuristic, and a user-selected Original backend remains selected.

The loopback server binds only to `127.0.0.1`, validates clip IDs through SQLite, and
serves standards-compliant byte ranges. No source clip is uploaded or exposed on the
LAN. Playback shortcuts are Space, Left/Right, Shift+Left/Right, and I/O.

Useful checks:

```bash
npm run check
npm test
npm run build
npm run check:rust
npm run test:rust
```

`npm run build:desktop` copies FFmpeg and FFprobe from `PATH` and creates the native
bundle. Official releases instead download static x86-64 builds in CI so friends do
not need to install FFmpeg separately.

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
[`cloud/wrangler.jsonc`](cloud/wrangler.jsonc). The config expects these domains:

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
[`cloud/.dev.vars.example`](cloud/.dev.vars.example).

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

### 7. Configure signed releases and live updates

Generate a Tauri updater keypair on a trusted machine:

```bash
npm run tauri -- signer generate --write-keys ~/.tauri/clip-engine.key
```

Add these GitHub repository secrets:

- `TAURI_SIGNING_PRIVATE_KEY`: contents of the private key.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password.
- `TAURI_UPDATER_PUBLIC_KEY`: the generated public key.

Keep the private key backed up and offline; losing it prevents existing installations
from accepting future updates. Never commit it. The public key is injected into each
release build through `TAURI_CONFIG`.

Bump the version in both `package.json` and `src-tauri/tauri.conf.json`, then push a
version tag:

```bash
git tag v1.0.1
git push origin v1.0.1
```

The **Desktop release** workflow builds signed-update artifacts and Windows/Linux
installers, then creates a draft GitHub Release. Test both installers and publish the
draft. Every running app checks the release feed shortly after launch; an available
version appears as a one-click update button and restarts after installation.
GitHub Releases must remain publicly downloadable for this configured update endpoint;
if the repository is private, mirror `latest.json`, installers, and signatures to a
public HTTPS origin and change the updater endpoint before the first release.

Windows Authenticode signing is separate from Tauri update signing. Add certificate
configuration before broad distribution if you want to avoid SmartScreen reputation
warnings. Linux package signing can likewise be added for an apt repository, but the
signed Tauri updater already authenticates application updates.

## Storage and deletion behavior

Local originals are never deleted by Clip Engine. Removing a local clip deletes only
its preview, exports, and SQLite history. Deleting a published version deletes its R2
video and thumbnail plus the local export. Extending a clip rewrites its two R2 objects
and advances its D1 expiry by another 30 days.

Desktop data is stored in the platform application-data directory:

- Windows: `%LOCALAPPDATA%\\dev.dab.clip-engine`
- Linux: `$XDG_DATA_HOME/dev.dab.clip-engine` or `~/.local/share/dev.dab.clip-engine`

The SQLite database is the only irreplaceable local app file. Originals remain wherever
the user recorded them. The cloud data plane is small enough that Workers, D1, and R2
are normally simpler and cheaper than exposing a home server; the home server is better
used for encrypted backups of the repository, signing key, and optional source clips.

## Production checklist

- Replace every `replace-with-*` value in `cloud/wrangler.jsonc`.
- Verify all three custom domains and HTTPS before distributing installers.
- Apply `cloud/lifecycle.json` and confirm it with `wrangler r2 bucket lifecycle list`.
- Store Worker and GitHub secrets only in their respective secret stores.
- Test owner sign-in, account approval, password reset, revocation, multipart upload, expiry, and
  one signed update using a staging account first.
- Enable GitHub branch protection and dependency/security update automation.
- Keep the Tauri signing private key in at least two encrypted offline backups.

## Security notes

See [`docs/SECURITY.md`](docs/SECURITY.md) for the trust boundaries, credential model,
revocation behavior, and an operational incident checklist.
