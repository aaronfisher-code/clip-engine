# Bundled OBS runtime

Release preparation populates this directory with the pinned libobs runtime,
OBS data files, capture plugins, encoder plugins, and the `obs-ffmpeg-mux`
helper. On Linux the runtime must include `obs-ffmpeg.so`, `obs-nvenc.so`, and
`obs-qsv11.so`; on Windows it must include the corresponding `.dll` files.
`obs-ffmpeg` provides VAAPI encoders on Linux and AMD AMF encoders on Windows,
so there is no separate AMD encoder module to bundle. Use
`scripts/prepare-libobs-runtime.mjs` with a checksum-verified archive before
creating an installer; the repository intentionally does not commit platform
runtime binaries.

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

Linux runtimes that advertise per-application audio must include the
`linux-pipewire-audio` plugin under `obs-plugins/` plus its matching locale
data under `data/obs-plugins/linux-pipewire-audio/`. The helper uses that
plugin to link Spotify, Discord, games, and other PipeWire application streams
to independent OBS tracks and, when requested, build a system track that
excludes selected application streams; system and microphone capture do not
depend on it.

Windows playback-device tracks use the standard `wasapi_output_capture` source
and Core Audio render-endpoint IDs, so no additional OBS plugin is required.
The endpoint itself is mixed: use Voicemeeter or Windows per-app output routing
to place applications on separate virtual devices, and disable the default
System audio route when overlap is unwanted. Recreated virtual devices can
produce stale saved IDs and must be refreshed and added again; Windows process
subtraction is not provided by this endpoint-based path. User-defined audio
route names are passed to the FFmpeg muxer as stream titles.
