# Bundled native tools

Release builds download static x86-64 GPL FFmpeg and FFprobe from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) into this directory.

Windows releases also copy `mpv-2.dll` from an official libmpv build. The installer
places that DLL next to `clip-engine.exe` so Windows can load it. Linux AppImage
and deb packages ship `libmpv.so.2` next to the binary (and AppImage also embeds it)
so friends do not need a distro package. The Linux binary is linked with an `$ORIGIN`
rpath so the bundled library is found without installing `libmpv`.

Local `cargo run` uses `ffmpeg`/`ffprobe` and `libmpv` from the system. Generated
binaries are gitignored.
