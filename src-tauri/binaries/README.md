# Release media tools

The release workflow downloads the current static x86-64 GPL FFmpeg and FFprobe builds
into this directory before packaging. Local desktop builds copy the executables found
on `PATH`; the generated binaries are intentionally ignored by Git.
