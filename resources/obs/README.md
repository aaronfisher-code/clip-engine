# Bundled OBS runtime

Release preparation populates this directory with the pinned libobs runtime,
OBS data files, capture plugins, encoder plugins, and the `obs-ffmpeg-mux`
helper. The preparation script verifies the platform capture modules before
installing the runtime. Linux requires `linux-capture.so`,
`linux-pipewire.so`, and `linux-pulseaudio.so`; Windows requires
`win-capture.dll` and `win-wasapi.dll`. Both platforms require
`obs-ffmpeg`, `obs-nvenc`, and `obs-qsv11` for the supported encoder matrix.
`obs-ffmpeg` also provides the replay-buffer output and audio encoders, as well
as VAAPI encoders on Linux and AMD AMF encoders on Windows, so there is no
separate AMD encoder module to bundle. Use
`scripts/prepare-libobs-runtime.mjs` with a checksum-verified archive before
creating an installer; the repository intentionally does not commit platform
runtime binaries.
The runtime also retains the NVENC and Quick Sync capability-test helpers;
the recorder stages them beside its executable before libobs starts so OBS can
perform hardware capability detection.
The Linux preparation step removes the OBS desktop executable because the
recorder uses libobs directly and does not need the Qt-based OBS frontend. It
also removes the unused OBS WebSocket module so AppImage packaging does not
introduce its optional QR-code library dependency.

The desktop application launches its bundled recorder helper with this runtime;
end users do not need to install OBS Studio separately. Hardware encoder
support remains dependent on the host GPU and its vendor driver. The helper
stages `obs-ffmpeg-mux` beside the recorder when running from a development
checkout; packaged builds include it beside the recorder because OBS resolves
the mux helper relative to its executable. The helper discovers the actual
encoder properties at startup so Automatic and Advanced recorder settings
remain compatible with the bundled OBS build.

Linux accepts either the flattened `data/` plus `obs-plugins/` layout or the
standard OBS install layout (`share/obs/` plus `lib/obs-plugins/`). The runtime
preparation script supplements a Linux archive missing `obs-nvenc.so` or
`obs-qsv11.so` from the build host's OBS plugin package when available, then
fails rather than silently shipping a runtime without the required encoder
modules. The host still supplies the matching NVIDIA driver, Intel oneVPL/VAAPI
runtime, or AMD Mesa/libva driver; those GPU drivers must not be copied into the
application runtime.

Linux release runtimes also include the
`linux-pipewire-audio` plugin under `obs-plugins/` plus its matching locale
data under `data/obs-plugins/linux-pipewire-audio/locale/`. Release CI
downloads and checksum-verifies that third-party plugin separately, then
installs it into the pinned runtime. The helper uses it to link Spotify,
Discord, games, and other PipeWire application streams to independent OBS
tracks and, when requested, build a system track that excludes selected
application streams. System and microphone capture remain available through
the native PipeWire or PulseAudio modules when the optional application plugin
is unavailable.

Windows playback-device tracks use the standard `wasapi_output_capture` source
and Core Audio render-endpoint IDs, so no additional OBS plugin is required.
The endpoint itself is mixed: use Voicemeeter or Windows per-app output routing
to place applications on separate virtual devices, and disable the default
System audio route when overlap is unwanted. Recreated virtual devices can
produce stale saved IDs and must be refreshed and added again; Windows process
subtraction is not provided by this endpoint-based path. User-defined audio
route names are passed to the FFmpeg muxer as stream titles.
