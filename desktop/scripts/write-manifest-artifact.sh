#!/usr/bin/env bash
#
# Appends (or initialises) an artifact entry in dist/release/desktop-update.manifest.json.
#
# Usage:
#   write-manifest-artifact.sh <version> <platform> <arch> <url> <sha256> <size>
#
# If the manifest doesn't exist yet it is created.
# If it already exists the artifact is appended to the artifacts array.
# Duplicate platform+arch entries are replaced.

set -euo pipefail

if [[ $# -ne 6 ]]; then
  echo "Usage: $0 <version> <platform> <arch> <url> <sha256> <size>" >&2
  exit 1
fi

version="$1"
platform="$2"
arch="$3"
url="$4"
sha256="$5"
size="$6"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
manifest_dir="${repo_root}/dist/release"
manifest_path="${manifest_dir}/desktop-update.manifest.json"

tag="v${version}"
github_repository="${GITHUB_REPOSITORY:-joris-gallot/reviu}"
release_notes_url="https://github.com/${github_repository}/releases/tag/${tag}"

new_artifact="$(cat <<JSON
{
  "platform": "${platform}",
  "arch": "${arch}",
  "url": "${url}",
  "sha256": "${sha256}",
  "size": ${size}
}
JSON
)"

mkdir -p "${manifest_dir}"

if [[ -f "${manifest_path}" ]]; then
  # Update version/releaseNotesUrl, remove existing entry for same platform+arch, then append new artifact
  updated="$(jq \
    --arg version "${version}" \
    --arg releaseNotesUrl "${release_notes_url}" \
    --arg platform "${platform}" \
    --arg arch "${arch}" \
    --argjson artifact "${new_artifact}" \
    '
      .version = $version
      | .releaseNotesUrl = $releaseNotesUrl
      | .artifacts = ([.artifacts[] | select(.platform != $platform or .arch != $arch)] + [$artifact])
      | .artifacts |= sort_by(.platform, .arch)
    ' "${manifest_path}"
  )"
  printf '%s\n' "${updated}" > "${manifest_path}"
else
  cat > "${manifest_path}" <<EOF
{
  "version": "${version}",
  "minimumSupportedVersion": "0.0.0",
  "releaseNotesUrl": "${release_notes_url}",
  "artifacts": [
    ${new_artifact}
  ]
}
EOF
fi

echo "Manifest updated: ${manifest_path} (${platform}/${arch})"
