#!/usr/bin/env bash
set -euo pipefail

DEFAULT_MANIFEST_URL="https://github.com/joris-gallot/reviu/releases/latest/download/desktop-update.manifest.json"
DEFAULT_INSTALL_BASE="${XDG_DATA_HOME:-${HOME}/.local/share}/reviu"
DEFAULT_BIN_DIR="${HOME}/.local/bin"
DEFAULT_APPLICATIONS_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/applications"
DEFAULT_ICONS_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/icons/hicolor/512x512/apps"

usage() {
  cat <<EOF
Usage: $0 [--arch x86_64|aarch64] [--manifest-url URL]

Install the latest Linux build of Reviu for the current user.

Options:
  --arch ARCH          Override detected architecture
  --manifest-url URL   Override the desktop update manifest URL
  --help, -h           Show this help
EOF
}

die() {
  echo "Error: $*" >&2
  exit 1
}

log() {
  echo "==> $*"
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "'$1' is required"
}

download_to_file() {
  local url="$1"
  local output_path="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "${url}" -o "${output_path}"
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    wget -qO "${output_path}" "${url}"
    return
  fi

  die "Could not find 'curl' or 'wget' in your path"
}

normalize_linux_arch() {
  case "${1:-}" in
    x86_64|amd64)
      echo "x86_64"
      ;;
    aarch64|arm64)
      echo "aarch64"
      ;;
    *)
      return 1
      ;;
  esac
}

detect_linux_arch() {
  local machine
  machine="$(uname -m)"
  normalize_linux_arch "${machine}" || die "Unsupported Linux architecture: ${machine}"
}

sha256_check_cmd() {
  if command -v sha256sum >/dev/null 2>&1; then
    echo "sha256sum"
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    echo "shasum"
    return
  fi

  die "Either 'sha256sum' or 'shasum' is required"
}

extract_linux_artifact_from_manifest() {
  local manifest_path="$1"
  local arch="$2"

  python3 - "${manifest_path}" "${arch}" <<'PY'
import json
import os
import sys
from urllib.parse import urlparse

manifest_path, arch = sys.argv[1], sys.argv[2]
with open(manifest_path, "r", encoding="utf-8") as fh:
    manifest = json.load(fh)

version = manifest.get("version")
if not isinstance(version, str) or not version.strip():
    raise SystemExit("Manifest is missing a valid version")

artifact = None
for candidate in manifest.get("artifacts", []):
    if candidate.get("platform") == "linux" and candidate.get("arch") == arch:
      artifact = candidate
      break

if artifact is None:
    raise SystemExit(f"No linux artifact found for arch={arch}")

url = artifact.get("url")
sha256 = artifact.get("sha256")
size = artifact.get("size")
if not isinstance(url, str) or not url:
    raise SystemExit("Artifact url is missing")
if not isinstance(sha256, str) or not sha256:
    raise SystemExit("Artifact sha256 is missing")
if not isinstance(size, int) or size <= 0:
    raise SystemExit("Artifact size is missing")

file_name = os.path.basename(urlparse(url).path)
if not file_name:
    raise SystemExit("Artifact file name is missing")

print("\t".join([
    version,
    url,
    sha256.lower(),
    str(size),
    file_name,
]))
PY
}

verify_archive_checksum() {
  local archive_path="$1"
  local expected_sha256="$2"
  local checksum_cmd="$3"
  local actual_sha256

  if [[ "${checksum_cmd}" == "sha256sum" ]]; then
    actual_sha256="$(sha256sum "${archive_path}" | awk '{ print $1 }')"
  else
    actual_sha256="$(shasum -a 256 "${archive_path}" | awk '{ print $1 }')"
  fi

  if [[ "${actual_sha256}" != "${expected_sha256}" ]]; then
    die "Checksum mismatch for ${archive_path}"
  fi
}

write_desktop_entry() {
  local desktop_entry_path="$1"
  local binary_path="$2"
  local icon_path="$3"

  cat > "${desktop_entry_path}" <<EOF
[Desktop Entry]
Type=Application
Name=Reviu
Comment=Keyboard-first Git client
Exec=${binary_path} %U
Icon=${icon_path}
Terminal=false
Categories=Development;VersionControl;
MimeType=x-scheme-handler/reviu;
StartupWMClass=Reviu
EOF
}

print_path_help() {
  local binary_path="$1"
  local bin_dir
  bin_dir="$(dirname "${binary_path}")"

  if command -v reviu >/dev/null 2>&1 && [[ "$(command -v reviu)" == "${binary_path}" ]]; then
    log "Run with 'reviu'"
    return
  fi

  echo "To run Reviu from your terminal, add ${bin_dir} to your PATH."
  case "${SHELL:-}" in
    *zsh)
      echo "  echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.zshrc"
      echo "  source ~/.zshrc"
      ;;
    *fish)
      echo "  fish_add_path -U \$HOME/.local/bin"
      ;;
    *)
      echo "  echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.bashrc"
      echo "  source ~/.bashrc"
      ;;
  esac
  echo "To run Reviu now, '${binary_path}'"
}

main() {
  local manifest_url="${REVIU_MANIFEST_URL:-${DEFAULT_MANIFEST_URL}}"
  local install_base="${REVIU_INSTALL_BASE:-${DEFAULT_INSTALL_BASE}}"
  local bin_dir="${REVIU_BIN_DIR:-${DEFAULT_BIN_DIR}}"
  local applications_dir="${REVIU_APPLICATIONS_DIR:-${DEFAULT_APPLICATIONS_DIR}}"
  local icons_dir="${REVIU_ICONS_DIR:-${DEFAULT_ICONS_DIR}}"
  local arch=""

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --arch)
        [[ $# -ge 2 ]] || die "Missing value for --arch"
        arch="$(normalize_linux_arch "$2")" || die "Unsupported Linux architecture: $2"
        shift 2
        ;;
      --manifest-url)
        [[ $# -ge 2 ]] || die "Missing value for --manifest-url"
        manifest_url="$2"
        shift 2
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        usage >&2
        die "Unknown argument: $1"
        ;;
    esac
  done

  if [[ -z "${arch}" ]]; then
    arch="$(detect_linux_arch)"
  fi

  require_cmd tar
  require_cmd python3

  local checksum_cmd
  checksum_cmd="$(sha256_check_cmd)"

  local temp_dir=""
  temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/reviu-install.XXXXXX")"
  local manifest_path="${temp_dir}/desktop-update.manifest.json"
  local archive_path="${temp_dir}/reviu-linux.tar.gz"
  local extract_dir="${temp_dir}/extract"

  trap 'rm -rf "${temp_dir:-}"' EXIT

  log "Fetching release manifest"
  download_to_file "${manifest_url}" "${manifest_path}"

  local version
  local artifact_url
  local artifact_sha256
  local artifact_size
  local artifact_file_name
  local artifact_metadata
  artifact_metadata="$(extract_linux_artifact_from_manifest "${manifest_path}" "${arch}")"
  IFS=$'\t' read -r version artifact_url artifact_sha256 artifact_size artifact_file_name <<<"${artifact_metadata}"

  log "Downloading Reviu ${version} for Linux ${arch}"
  download_to_file "${artifact_url}" "${archive_path}"

  local downloaded_size
  downloaded_size="$(wc -c < "${archive_path}" | tr -d ' ')"
  if [[ "${downloaded_size}" != "${artifact_size}" ]]; then
    die "Archive size mismatch: expected ${artifact_size}, got ${downloaded_size}"
  fi

  verify_archive_checksum "${archive_path}" "${artifact_sha256}" "${checksum_cmd}"

  mkdir -p "${extract_dir}"
  tar -xzf "${archive_path}" -C "${extract_dir}"

  local package_root
  package_root="$(find "${extract_dir}" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
  [[ -n "${package_root}" ]] || die "Extracted archive is missing a top-level directory"

  local version_dir="${install_base}/versions/${version}"
  local current_link="${install_base}/current"
  local current_binary="${current_link}/bin/reviu"
  local installed_icon="${version_dir}/share/icons/hicolor/512x512/apps/reviu.png"
  local icon_target="${icons_dir}/reviu.png"
  local desktop_entry_path="${applications_dir}/reviu.desktop"

  [[ -f "${package_root}/bin/reviu" ]] || die "Extracted archive is missing bin/reviu"
  [[ -f "${package_root}/share/icons/hicolor/512x512/apps/reviu.png" ]] || die "Extracted archive is missing the app icon"

  mkdir -p "${install_base}/versions" "${bin_dir}" "${applications_dir}" "${icons_dir}"
  rm -rf "${version_dir}"
  mv "${package_root}" "${version_dir}"
  ln -sfn "${version_dir}" "${current_link}"
  ln -sfn "${current_binary}" "${bin_dir}/reviu"

  cp "${installed_icon}" "${icon_target}"
  write_desktop_entry "${desktop_entry_path}" "${current_binary}" "${icon_target}"

  if [[ "${REVIU_SKIP_DESKTOP_REGISTRATION:-0}" != "1" ]]; then
    if command -v update-desktop-database >/dev/null 2>&1; then
      update-desktop-database "${applications_dir}" >/dev/null 2>&1 || true
    fi

    if command -v xdg-mime >/dev/null 2>&1; then
      xdg-mime default reviu.desktop x-scheme-handler/reviu >/dev/null 2>&1 || true
    fi
  fi

  log "Installed Reviu ${version}"
  log "Binary: ${bin_dir}/reviu"
  log "Desktop entry: ${desktop_entry_path}"
  log "Archive: ${artifact_file_name}"
  print_path_help "${bin_dir}/reviu"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
