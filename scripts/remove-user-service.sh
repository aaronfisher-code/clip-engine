#!/usr/bin/env bash
set -euo pipefail

config_root="${XDG_CONFIG_HOME:-${HOME:?}/.config}"
unit_path="${config_root}/systemd/user/clip-engine.service"

if [[ "${unit_path}" != /* ]]; then
  echo "The systemd user service path must be absolute: ${unit_path}" >&2
  exit 1
fi
if [[ -e "${unit_path}" ]] && ! grep -q '^# Managed by Clip Engine\.' "${unit_path}"; then
  echo "Refusing to remove an unmanaged service: ${unit_path}" >&2
  exit 1
fi

systemctl --user disable --now clip-engine.service 2>/dev/null || true
if [[ -f "${unit_path}" ]]; then
  rm -f -- "${unit_path}"
  echo "Removed ${unit_path}"
fi
systemctl --user daemon-reload
echo "Clip Engine's service was removed. Local clips and configuration were kept."
