# Third-party notices

Release installers bundle unmodified command-line builds of FFmpeg and FFprobe from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds). Those binaries and their
source are available under the licenses listed by that project, including the GNU GPL.
Clip Engine invokes them as separate processes and does not link to FFmpeg libraries.

The desktop UI embeds Latin subsets of [IBM Plex Sans](https://github.com/IBM/plex) and
IBM Plex Mono, licensed under the SIL Open Font License.

Desktop builds also link dynamically against [libmpv](https://mpv.io/) for playback.
libmpv is available under GNU GPL/LGPL terms from the mpv project. Windows installers
include `mpv-2.dll`; Linux packages ship `libmpv`.

The optional recorder helper links against the pinned
[libobs-rs/libobs](https://github.com/libobs-rs/libobs-rs) bindings and loads the
matching [OBS Studio](https://github.com/obsproject/obs-studio) runtime and
plugins. OBS Studio is GPL-2.0 licensed. Release builds must include the
corresponding OBS license, notices, and source-offer files beside the bundled
runtime; `scripts/prepare-libobs-runtime.mjs` verifies the runtime archive
before installation.

The recorder uses [global-hotkey](https://github.com/tauri-apps/global-hotkey)
under its Apache-2.0/MIT license for Windows and X11 global replay hotkeys.
Desktop save notifications use [notify-rust](https://github.com/hoodie/notify-rust)
under its Apache-2.0/MIT license.
