# Bundled native tools

Release builds download static x86-64 GPL FFmpeg and FFprobe from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) into this directory.

Windows releases also copy `mpv-2.dll` from an official libmpv build. The installer
places that DLL next to `clip-engine.exe` so Windows can load it. Linux AppImage
and deb packages ship `libmpv` so friends do not need a distro package.

Local `cargo run` uses `ffmpeg`/`ffprobe` and `libmpv` from the system. Generated
binaries are gitignored.
