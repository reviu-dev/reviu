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

Build a Windows Inno Setup installer for Reviu.

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

resolve_windows_target_from_manifest_arch() {
  case "$1" in
    x86_64)
      echo "x86_64-pc-windows-msvc"
      ;;
    aarch64)
      echo "aarch64-pc-windows-msvc"
      ;;
    *)
      die "Unsupported manifest_arch: $1. Expected one of: x86_64, aarch64"
      ;;
  esac
}

resolve_inno_target_arch_from_manifest_arch() {
  case "$1" in
    x86_64)
      echo "x64"
      ;;
    aarch64)
      echo "arm64"
      ;;
    *)
      die "Unsupported manifest_arch: $1. Expected one of: x86_64, aarch64"
      ;;
  esac
}

resolve_windows_installer_version() {
  local version="$1"

  if [[ "${version}" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+) ]]; then
    printf '%s.%s.%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "${BASH_REMATCH[3]}"
    return
  fi

  die "Windows installer version must start with MAJOR.MINOR.PATCH: ${version}"
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

  if command -v certutil.exe >/dev/null 2>&1; then
    certutil.exe -hashfile "$(to_windows_path "$1")" SHA256 \
      | awk 'NR == 2 { gsub(/[[:space:]]/, "", $0); print tolower($0) }'
    return
  fi

  die "One of 'sha256sum', 'shasum' or 'certutil.exe' is required"
}

to_windows_path() {
  local path="$1"

  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "${path}"
    return
  fi

  printf '%s\n' "${path}"
}

to_bash_path() {
  local path="$1"

  if command -v cygpath >/dev/null 2>&1; then
    cygpath -u "${path}" 2>/dev/null && return
  fi

  printf '%s\n' "${path}"
}

resolve_inno_setup_compiler() {
  local candidate

  if [[ -n "${INNO_SETUP_COMPILER:-}" ]]; then
    candidate="$(to_bash_path "${INNO_SETUP_COMPILER}")"
    if [[ -x "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return
    fi
  fi

  if command -v ISCC >/dev/null 2>&1; then
    command -v ISCC
    return
  fi

  if command -v ISCC.exe >/dev/null 2>&1; then
    command -v ISCC.exe
    return
  fi

  for candidate in \
    "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
    "/c/Program Files/Inno Setup 6/ISCC.exe"; do
    if [[ -x "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return
    fi
  done

  die "Inno Setup compiler not found. Install Inno Setup 6.3+ or set INNO_SETUP_COMPILER."
}

WRITE_MANIFEST_ARTIFACT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/write-manifest-artifact.sh"

main() {
  if [[ $# -ne 2 ]]; then
    usage >&2
    exit 1
  fi

  local version="$1"
  local manifest_arch="$2"

  validate_manifest_arch "${manifest_arch}"

  local target
  target="$(resolve_windows_target_from_manifest_arch "${manifest_arch}")"

  local target_arch
  target_arch="$(resolve_inno_target_arch_from_manifest_arch "${manifest_arch}")"

  local installer_version
  installer_version="$(resolve_windows_installer_version "${version}")"

  local app_name="Reviu"
  local binary_name="reviu"
  local repo_root="${REVIU_REPO_ROOT}"
  local desktop_dir="${repo_root}/desktop"
  local github_repository="${GITHUB_REPOSITORY:-joris-gallot/reviu}"
  local api_base_url="${API_BASE_URL:-https://api.reviu.dev}"
  local tag="v${version}"
  local output_dir="${repo_root}/dist/release/windows/${target}"
  local installer_name="${app_name}-${version}-windows-${manifest_arch}.exe"
  local installer_path="${output_dir}/${installer_name}"
  local artifact_url="https://github.com/${github_repository}/releases/download/${tag}/${installer_name}"
  local icon_path="${desktop_dir}/crates/reviu/assets/reviu.ico"
  local binary_path="${desktop_dir}/target/${target}/release/${binary_name}.exe"
  local iss_path="${desktop_dir}/crates/reviu/resources/windows/reviu.iss"
  local iscc
  local installer_size
  local installer_sha256

  require_cmd cargo

  iscc="$(resolve_inno_setup_compiler)"

  if [[ ! -f "${icon_path}" ]]; then
    die "App icon not found: ${icon_path}"
  fi

  if [[ ! -f "${iss_path}" ]]; then
    die "Inno Setup script not found: ${iss_path}"
  fi

  if [[ "${REVIU_SKIP_BUILD:-0}" != "1" ]]; then
    log "Building ${app_name} ${version} for Windows ${manifest_arch} (${target})"
    (
      cd "${desktop_dir}"
      API_BASE_URL="${api_base_url}" cargo build -p reviu --release --target "${target}"
    )
  else
    log "Skipping cargo build"
  fi

  if [[ ! -f "${binary_path}" ]]; then
    die "Expected Windows binary not found: ${binary_path}"
  fi

  rm -rf "${output_dir}"
  mkdir -p "${output_dir}"

  log "Creating Windows installer"
  "${iscc}" \
    "$(to_windows_path "${iss_path}")" \
    "/dAppName=${app_name}" \
    "/dAppExeName=${binary_name}" \
    "/dVersion=${version}" \
    "/dVersionInfoVersion=${installer_version}" \
    "/dOutputDir=$(to_windows_path "${output_dir}")" \
    "/dAppSetupName=${app_name}-${version}-windows-${manifest_arch}" \
    "/dSourceDir=$(to_windows_path "${repo_root}")" \
    "/dBinaryPath=$(to_windows_path "${binary_path}")" \
    "/dIconPath=$(to_windows_path "${icon_path}")" \
    "/dTargetArch=${target_arch}"

  installer_size="$(wc -c < "${installer_path}" | tr -d '[:space:]')"
  installer_sha256="$(sha256_hex "${installer_path}")"

  bash "${WRITE_MANIFEST_ARTIFACT}" \
    "${version}" \
    "windows" \
    "${manifest_arch}" \
    "${artifact_url}" \
    "${installer_sha256}" \
    "${installer_size}"

  log "Created ${installer_path}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
