#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REVIU_REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ENV_FILE="${SCRIPT_DIR}/.env"

if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
fi

usage() {
  cat <<EOF
Usage: $0 <version> <manifest_arch>

Build a Linux release archive for Reviu.

Arguments:
  version        Release version without the leading v
  manifest_arch  Target architecture: x86_64 or aarch64
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

is_supported_manifest_arch() {
  case "$1" in
    x86_64|aarch64)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

validate_manifest_arch() {
  local manifest_arch="$1"

  if ! is_supported_manifest_arch "${manifest_arch}"; then
    die "Unsupported manifest_arch: ${manifest_arch}. Expected one of: x86_64, aarch64"
  fi
}

resolve_linux_target_from_manifest_arch() {
  case "$1" in
    x86_64)
      echo "x86_64-unknown-linux-gnu"
      ;;
    aarch64)
      echo "aarch64-unknown-linux-gnu"
      ;;
    *)
      die "Unsupported manifest_arch: $1. Expected one of: x86_64, aarch64"
      ;;
  esac
}

sha256_hex() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{ print $1 }'
    return
  fi

  die "Either 'sha256sum' or 'shasum' is required"
}

write_metadata_manifest() {
  local metadata_path="$1"
  local version="$2"
  local release_notes_url="$3"
  local manifest_arch="$4"
  local artifact_url="$5"
  local sha256="$6"
  local size="$7"

  cat > "${metadata_path}" <<EOF
{
  "version": "${version}",
  "minimumSupportedVersion": "0.0.0",
  "releaseNotesUrl": "${release_notes_url}",
  "artifacts": [
    {
      "platform": "linux",
      "arch": "${manifest_arch}",
      "url": "${artifact_url}",
      "sha256": "${sha256}",
      "size": ${size}
    }
  ]
}
EOF
}

main() {
  if [[ $# -ne 2 ]]; then
    usage >&2
    exit 1
  fi

  local version="$1"
  local manifest_arch="$2"

  validate_manifest_arch "${manifest_arch}"

  local target
  target="$(resolve_linux_target_from_manifest_arch "${manifest_arch}")"

  local app_name="Reviu"
  local binary_name="reviu"
  local repo_root="${REVIU_REPO_ROOT}"
  local desktop_dir="${repo_root}/desktop"
  local github_repository="${GITHUB_REPOSITORY:-joris-gallot/reviu}"
  local api_base_url="${API_BASE_URL:-https://api.reviu.dev}"
  local tag="v${version}"
  local output_dir="${repo_root}/dist/release/linux/${target}"
  local archive_name="${app_name}-${version}-linux-${manifest_arch}.tar.gz"
  local archive_path="${output_dir}/${archive_name}"
  local metadata_path="${output_dir}/desktop-update.manifest.json"
  local artifact_url="https://github.com/${github_repository}/releases/download/${tag}/${archive_name}"
  local release_notes_url="https://github.com/${github_repository}/releases/tag/${tag}"
  local icon_source_path="${desktop_dir}/crates/reviu/assets/reviu_icon.png"
  local binary_path="${desktop_dir}/target/${target}/release/${binary_name}"
  local staging_dir=""
  staging_dir="$(mktemp -d "${TMPDIR:-/tmp}/reviu-linux-package.XXXXXX")"
  local package_root="${staging_dir}/${app_name}-${version}-linux-${manifest_arch}"
  local archive_size
  local archive_sha256

  trap 'rm -rf "${staging_dir:-}"' EXIT

  require_cmd cargo
  require_cmd tar

  if [[ ! -f "${icon_source_path}" ]]; then
    die "App icon not found: ${icon_source_path}"
  fi

  if [[ "${REVIU_SKIP_BUILD:-0}" != "1" ]]; then
    log "Building ${app_name} ${version} for Linux ${manifest_arch} (${target})"
    (
      cd "${desktop_dir}"
      API_BASE_URL="${api_base_url}" cargo build -p reviu --release --target "${target}"
    )
  else
    log "Skipping cargo build"
  fi

  if [[ ! -f "${binary_path}" ]]; then
    die "Expected Linux binary not found: ${binary_path}"
  fi

  rm -rf "${output_dir}"
  mkdir -p "${output_dir}"
  mkdir -p \
    "${package_root}/bin" \
    "${package_root}/share/icons/hicolor/512x512/apps"

  cp "${binary_path}" "${package_root}/bin/${binary_name}"
  cp "${icon_source_path}" "${package_root}/share/icons/hicolor/512x512/apps/reviu.png"

  log "Creating Linux release archive"
  tar -C "${staging_dir}" -czf "${archive_path}" "$(basename "${package_root}")"

  archive_size="$(wc -c < "${archive_path}" | tr -d ' ')"
  archive_sha256="$(sha256_hex "${archive_path}")"

  write_metadata_manifest \
    "${metadata_path}" \
    "${version}" \
    "${release_notes_url}" \
    "${manifest_arch}" \
    "${artifact_url}" \
    "${archive_sha256}" \
    "${archive_size}"

  log "Created ${archive_path}"
  log "Created ${metadata_path}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
