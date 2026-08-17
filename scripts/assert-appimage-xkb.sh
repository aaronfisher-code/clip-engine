#!/usr/bin/env bash
# Fail if an AppImage still embeds XKB libraries. Mixing Ubuntu-bundled
# libxkbcommon with a host libxkbcommon-x11 SIGSEGVs on the first keypress.
set -euo pipefail

root="${1:-dist/desktop}"
mapfile -t images < <(find "${root}" -maxdepth 3 -iname '*.AppImage' -print | sort)
if [[ "${#images[@]}" -eq 0 ]]; then
  echo "No AppImage found under ${root}" >&2
  exit 1
fi

workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT

failed=0
for image in "${images[@]}"; do
  image="$(realpath "${image}")"
  echo "Checking ${image}"
  extract="${workdir}/$(basename "${image}").root"
  mkdir -p "${extract}"
  (
    cd "${extract}"
    chmod +x "${image}"
    "${image}" --appimage-extract >/dev/null
  )
  matches="$(find "${extract}" -type f \( -name 'libxkbcommon.so*' -o -name 'libxkbcommon-x11.so*' \) -print || true)"
  if [[ -n "${matches}" ]]; then
    echo "AppImage still bundles XKB libraries:" >&2
    echo "${matches}" >&2
    failed=1
  fi
done

if [[ "${failed}" -ne 0 ]]; then
  echo "Exclude libxkbcommon from the AppImage (package.metadata.packager.appimage.excluded-libraries)." >&2
  exit 1
fi
echo "No bundled libxkbcommon libraries."
