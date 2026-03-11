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
Usage: $0 [--no-notarize] <version> <arch> <target>

Build a signed macOS app bundle and DMG for Reviu.

Options:
  --no-notarize  Skip notarization, stapling and Gatekeeper assessment
  --help, -h     Show this help
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

require_env() {
  local missing=()
  local var_name
  for var_name in "$@"; do
    if [[ -z "${!var_name:-}" ]]; then
      missing+=("${var_name}")
    fi
  done

  if [[ "${#missing[@]}" -gt 0 ]]; then
    printf 'Missing required environment variables: %s\n' "${missing[*]}" >&2
    exit 1
  fi
}

unlock_signing_keychain() {
  security unlock-keychain -p "${APPLE_KEYCHAIN_PASSWORD}" "${APPLE_KEYCHAIN_PATH}"
}

assert_signing_identity() {
  local identities_output
  identities_output="$(security find-identity -v -p codesigning "${APPLE_KEYCHAIN_PATH}" 2>&1 || true)"
  if ! grep -F "${APPLE_SIGNING_IDENTITY}" <<<"${identities_output}" >/dev/null 2>&1; then
    printf '%s\n' "${identities_output}" >&2
    die "Signing identity not found in keychain: ${APPLE_SIGNING_IDENTITY}"
  fi
}

embed_app_icon() {
  local app_path="$1"
  local icon_source_path="$2"
  local app_name="$3"
  local plist_buddy="/usr/libexec/PlistBuddy"
  local plist_path="${app_path}/Contents/Info.plist"
  local resources_dir="${app_path}/Contents/Resources"

  if [[ ! -f "${icon_source_path}" ]]; then
    die "App icon not found: ${icon_source_path}"
  fi

  if [[ ! -x "${plist_buddy}" ]]; then
    die "PlistBuddy is required to embed the app icon"
  fi

  mkdir -p "${resources_dir}"
  cp "${icon_source_path}" "${resources_dir}/${app_name}.icns"

  "${plist_buddy}" -c "Delete :CFBundleIconFile" "${plist_path}" >/dev/null 2>&1 || true
  "${plist_buddy}" -c "Add :CFBundleIconFile string ${app_name}" "${plist_path}"
}

json_field() {
  /usr/bin/python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"
}

apply_dmg_layout() {
  local volume_name="$1"
  local app_name="$2"

  if [[ "${REVIU_SKIP_DMG_LAYOUT:-0}" == "1" ]]; then
    log "Skipping Finder layout customization"
    return
  fi

  if [[ ! -x "/usr/bin/osascript" ]]; then
    log "Skipping Finder layout customization"
    return
  fi

  log "Applying Finder layout"
  if ! /usr/bin/osascript <<EOF
tell application "Finder"
  tell disk "${volume_name}"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set bounds of container window to {120, 120, 660, 440}
    set opts to the icon view options of container window
    set arrangement of opts to not arranged
    set icon size of opts to 128
    set text size of opts to 12
    set position of item "${app_name}.app" to {150, 180}
    set position of item "Applications" to {390, 180}
    update without registering applications
    delay 1
    close
  end tell
end tell
EOF
  then
    echo "Warning: Finder layout customization failed; continuing without layout tweaks" >&2
  fi
}

submit_and_validate_notarization() {
  local artifact_path="$1"
  local artifact_label="$2"
  local submit_output_path="$3"
  local log_output_path="$4"

  if [[ "${REVIU_DRY_RUN:-0}" == "1" ]]; then
    printf '{"id":"dry-run","status":"Accepted"}\n' > "${submit_output_path}"
  else
    if ! xcrun notarytool submit "${artifact_path}" \
      --apple-id "${APPLE_NOTARYTOOL_APPLE_ID}" \
      --password "${APPLE_NOTARYTOOL_APP_PASSWORD}" \
      --team-id "${APPLE_NOTARYTOOL_TEAM_ID}" \
      --wait \
      --output-format json | tee "${submit_output_path}"; then
      echo "notarytool submit failed for ${artifact_label}" >&2
      exit 1
    fi
  fi

  local submission_id
  submission_id="$(json_field "${submit_output_path}" id)"
  local submission_status
  submission_status="$(json_field "${submit_output_path}" status)"

  echo "Notarization status for ${artifact_label}: ${submission_status} (submission ${submission_id})"

  if [[ "${submission_status}" != "Accepted" ]]; then
    echo "Fetching Apple notarization log for ${artifact_label}..." >&2
    if xcrun notarytool log "${submission_id}" \
      --apple-id "${APPLE_NOTARYTOOL_APPLE_ID}" \
      --password "${APPLE_NOTARYTOOL_APP_PASSWORD}" \
      --team-id "${APPLE_NOTARYTOOL_TEAM_ID}" \
      "${log_output_path}"; then
      echo "Apple notarization log for ${artifact_label}:"
      cat "${log_output_path}"
    else
      echo "Unable to download Apple notarization log for ${artifact_label}." >&2
    fi
    exit 1
  fi
}

main() {
  local notarize=1

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --no-notarize)
        notarize=0
        shift
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      --)
        shift
        break
        ;;
      -*)
        usage >&2
        die "Unknown option: $1"
        ;;
      *)
        break
        ;;
    esac
  done

  if [[ $# -ne 3 ]]; then
    usage
    exit 1
  fi

  local version="$1"
  local arch="$2"
  local target="$3"
  local tag="v${version}"
  local app_name="Reviu"
  local volume_name="Reviu"
  local repo_root="${REVIU_REPO_ROOT}"
  local desktop_dir="${repo_root}/desktop"
  local runner_temp="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
  local app_path="${desktop_dir}/target/${target}/release/bundle/osx/${app_name}.app"
  local icon_source_path="${desktop_dir}/crates/reviu/assets/reviu.icns"
  local output_dir="${repo_root}/dist/release/macos/${target}"
  local dmg_name="${app_name}-${version}-macos-${arch}.dmg"
  local dmg_path="${output_dir}/${dmg_name}"
  local metadata_path="${output_dir}/desktop-update.${arch}.json"
  local artifact_url="https://github.com/${GITHUB_REPOSITORY}/releases/download/${tag}/${dmg_name}"
  local app_zip_path="${runner_temp}/${app_name}-${target}.app.zip"
  local app_submit_output_path="${runner_temp}/${app_name}-${target}.app.notary-submit.json"
  local app_notary_log_path="${runner_temp}/${app_name}-${target}.app.notary-log.json"
  local dmg_submit_output_path="${runner_temp}/${app_name}-${target}.dmg.notary-submit.json"
  local dmg_notary_log_path="${runner_temp}/${app_name}-${target}.dmg.notary-log.json"
  local dmg_root
  local rw_dmg_path
  local attach_info
  local attach_line
  local device=""

  require_cmd hdiutil
  require_cmd codesign
  require_cmd security
  require_cmd shasum
  require_cmd xcrun

  if [[ "${notarize}" == "1" ]]; then
    require_cmd spctl
  fi

  if [[ "${REVIU_SKIP_BUNDLE_BUILD:-0}" != "1" ]]; then
    require_cmd cargo
    if ! cargo bundle --version >/dev/null 2>&1; then
      die "cargo-bundle not found. Install it with: cargo install cargo-bundle"
    fi

    log "Building ${app_name} ${version} for ${arch} (${target})"
    (
      cd "${desktop_dir}"
      cargo bundle -p reviu --release --target "${target}"
    )
  else
    log "Skipping cargo bundle build"
  fi

  require_env \
    APPLE_KEYCHAIN_PATH \
    APPLE_KEYCHAIN_PASSWORD \
    APPLE_SIGNING_IDENTITY \
    APPLE_NOTARYTOOL_APPLE_ID \
    APPLE_NOTARYTOOL_APP_PASSWORD \
    APPLE_NOTARYTOOL_TEAM_ID \
    GITHUB_REPOSITORY

  if [[ ! -d "${app_path}" ]]; then
    die "Expected app bundle not found: ${app_path}"
  fi

  embed_app_icon "${app_path}" "${icon_source_path}" "${app_name}"

  rm -rf "${output_dir}"
  mkdir -p "${output_dir}"

  if [[ "${REVIU_DRY_RUN:-0}" != "1" ]]; then
    unlock_signing_keychain
    assert_signing_identity

    log "Signing app bundle"
    /usr/bin/codesign \
      --force \
      --deep \
      --options runtime \
      --timestamp \
      --sign "${APPLE_SIGNING_IDENTITY}" \
      --keychain "${APPLE_KEYCHAIN_PATH}" \
      "${app_path}"

    /usr/bin/codesign --verify --deep --strict --verbose=2 "${app_path}"
  else
    log "Dry run: skipping app codesign"
  fi

  if [[ "${notarize}" == "1" ]]; then
    rm -f "${app_zip_path}" "${app_submit_output_path}" "${app_notary_log_path}"
    if [[ "${REVIU_DRY_RUN:-0}" == "1" ]]; then
      : > "${app_zip_path}"
    else
      ditto -c -k --keepParent "${app_path}" "${app_zip_path}"
    fi

    submit_and_validate_notarization \
      "${app_zip_path}" \
      "${app_name}.app.zip (${target})" \
      "${app_submit_output_path}" \
      "${app_notary_log_path}"

    if [[ "${REVIU_DRY_RUN:-0}" != "1" ]]; then
      xcrun stapler staple "${app_path}"
      xcrun stapler validate "${app_path}"
    fi
    rm -f "${app_zip_path}"
  else
    log "Skipping app notarization"
  fi

  dmg_root="$(mktemp -d "${TMPDIR:-/tmp}/reviu-dmg-root.XXXXXX")"
  rw_dmg_path="${runner_temp}/${app_name}-${target}-rw.dmg"
  rm -f "${rw_dmg_path}"
  cp -R "${app_path}" "${dmg_root}/${app_name}.app"
  ln -s /Applications "${dmg_root}/Applications"

  if [[ "${REVIU_DRY_RUN:-0}" == "1" ]]; then
    : > "${dmg_path}"
  else
    log "Creating DMG staging image"
    hdiutil create \
      -volname "${volume_name}" \
      -srcfolder "${dmg_root}" \
      -ov \
      -fs HFS+ \
      -format UDRW \
      "${rw_dmg_path}" >/dev/null

    attach_info="$(hdiutil attach -readwrite -noverify -noautoopen "${rw_dmg_path}")"
    attach_line="$(printf '%s\n' "${attach_info}" | awk '/\/Volumes\// {print; exit}')"
    device="${attach_line%%[[:space:]]*}"
    if [[ -z "${device}" ]]; then
      die "Failed to mount temporary DMG. hdiutil output: ${attach_info}"
    fi

    apply_dmg_layout "${volume_name}" "${app_name}"

    hdiutil detach "${device}" -quiet
    device=""

    log "Converting DMG"
    hdiutil convert \
      "${rw_dmg_path}" \
      -format UDZO \
      -imagekey zlib-level=9 \
      -o "${dmg_path}" >/dev/null
    rm -f "${rw_dmg_path}"
  fi

  rm -rf "${dmg_root}"

  if [[ "${REVIU_DRY_RUN:-0}" != "1" ]]; then
    unlock_signing_keychain

    log "Signing DMG"
    /usr/bin/codesign \
      --force \
      --timestamp \
      --sign "${APPLE_SIGNING_IDENTITY}" \
      --keychain "${APPLE_KEYCHAIN_PATH}" \
      "${dmg_path}"

    /usr/bin/codesign --verify --verbose=2 "${dmg_path}"
  else
    log "Dry run: skipping DMG codesign"
  fi

  if [[ "${notarize}" == "1" ]]; then
    rm -f "${dmg_submit_output_path}" "${dmg_notary_log_path}"
    submit_and_validate_notarization \
      "${dmg_path}" \
      "${dmg_name}" \
      "${dmg_submit_output_path}" \
      "${dmg_notary_log_path}"

    if [[ "${REVIU_DRY_RUN:-0}" != "1" ]]; then
      xcrun stapler staple "${dmg_path}"
      xcrun stapler validate "${dmg_path}"
      /usr/sbin/spctl --assess --type open --context context:primary-signature --verbose=4 "${dmg_path}"
    fi
  else
    log "Skipping DMG notarization"
  fi

  local sha256
  sha256="$(shasum -a 256 "${dmg_path}" | awk '{print $1}')"
  local size
  size="$(wc -c < "${dmg_path}" | tr -d '[:space:]')"

  cat > "${metadata_path}" <<EOF
{
  "platform": "macos",
  "arch": "${arch}",
  "url": "${artifact_url}",
  "sha256": "${sha256}",
  "size": ${size}
}
EOF

  echo "Created DMG: ${dmg_path}"
  echo "Created metadata: ${metadata_path}"
}

main "$@"
