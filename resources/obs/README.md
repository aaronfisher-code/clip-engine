# Bundled OBS runtime

Release preparation populates this directory with the pinned libobs runtime,
OBS data files, capture plugins, and encoder plugins. Use
`scripts/prepare-libobs-runtime.mjs` with a checksum-verified archive before
creating an installer; the repository intentionally does not commit platform
runtime binaries.
