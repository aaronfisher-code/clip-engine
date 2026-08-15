#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_dir="$(cd -- "${script_dir}/.." && pwd -P)"
template_path="${repo_dir}/systemd/clip-engine.service.in"
node_path="$(command -v node)"
config_root="${XDG_CONFIG_HOME:-${HOME:?}/.config}"
unit_dir="${config_root}/systemd/user"
unit_path="${unit_dir}/clip-engine.service"

if [[ "${repo_dir}" == *$'\n'* || "${repo_dir}" == *'|'* ]]; then
  echo "Clip Engine's path contains unsupported characters: ${repo_dir}" >&2
  exit 1
fi
if [[ "${node_path}" == *$'\n'* || "${node_path}" == *'|'* ]]; then
  echo "Node's path contains unsupported characters: ${node_path}" >&2
  exit 1
fi
if [[ "${unit_dir}" != /* ]]; then
  echo "The systemd user configuration directory must be absolute: ${unit_dir}" >&2
  exit 1
fi
if [[ ! -f "${repo_dir}/.env" ]]; then
  echo "Missing ${repo_dir}/.env. Copy .env.example to .env and configure it first." >&2
  exit 1
fi
if [[ -e "${unit_path}" ]] && ! grep -q '^# Managed by Clip Engine\.' "${unit_path}"; then
  echo "Refusing to replace an existing unmanaged service: ${unit_path}" >&2
  exit 1
fi

echo "Building Clip Engine for production…"
(cd -- "${repo_dir}" && npm run build)

mkdir -p -- "${unit_dir}"
temporary_path="$(mktemp /tmp/clip-engine-service.XXXXXX)"
cleanup() {
  if [[ -n "${temporary_path:-}" && -f "${temporary_path}" && "${temporary_path}" == /tmp/clip-engine-service.* ]]; then
    rm -f -- "${temporary_path}"
  fi
}
trap cleanup EXIT

escaped_repo="${repo_dir//&/\\&}"
escaped_node="${node_path//&/\\&}"
sed \
  -e "s|@REPO_DIR@|${escaped_repo}|g" \
  -e "s|@NODE_PATH@|${escaped_node}|g" \
  "${template_path}" > "${temporary_path}"
install -m 0644 -- "${temporary_path}" "${unit_path}"

systemctl --user daemon-reload
systemctl --user enable --now clip-engine.service

echo
echo "Clip Engine is installed and running at http://127.0.0.1:4317"
echo "Status: npm run service:status"
echo "Logs:   npm run service:logs"
