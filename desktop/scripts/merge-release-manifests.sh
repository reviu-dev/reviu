#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REVIU_REPO_ROOT="${REVIU_REPO_ROOT:-$(cd "${SCRIPT_DIR}/../.." && pwd)}"

usage() {
  cat <<EOF
Usage: $0 <version> <manifest...>

Merge partial desktop release manifests into dist/release/desktop-update.manifest.json.

Arguments:
  version     Release version without the leading v
  manifest    One or more desktop-update.manifest.json files from platform builds
EOF
}

die() {
  echo "Error: $*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "'$1' is required"
}

if [[ $# -lt 2 ]]; then
  usage >&2
  exit 1
fi

version="$1"
shift

require_cmd jq

repo_root="${REVIU_REPO_ROOT}"
manifest_dir="${repo_root}/dist/release"
manifest_path="${manifest_dir}/desktop-update.manifest.json"
tag="v${version}"
github_repository="${GITHUB_REPOSITORY:-joris-gallot/reviu}"
release_notes_url="https://github.com/${github_repository}/releases/tag/${tag}"

for manifest in "$@"; do
  if [[ ! -f "${manifest}" ]]; then
    die "Manifest not found: ${manifest}"
  fi
done

mkdir -p "${manifest_dir}"

merged="$(jq -s \
  --arg version "${version}" \
  --arg releaseNotesUrl "${release_notes_url}" \
  '
    reduce .[] as $manifest (
      {
        version: $version,
        minimumSupportedVersion: "0.0.0",
        releaseNotesUrl: $releaseNotesUrl,
        artifacts: []
      };
      .minimumSupportedVersion = ($manifest.minimumSupportedVersion // .minimumSupportedVersion)
      | reduce (($manifest.artifacts // [])[]) as $artifact (
          .;
          .artifacts = (
            [.artifacts[] | select(.platform != $artifact.platform or .arch != $artifact.arch)]
            + [$artifact]
          )
        )
    )
    | .artifacts |= sort_by(.platform, .arch)
    | if (.artifacts | length) == 0 then error("merged manifest has no artifacts") else . end
  ' "$@")"

printf '%s\n' "${merged}" > "${manifest_path}"
echo "Manifest merged: ${manifest_path} ($(jq '.artifacts | length' <<<"${merged}") artifacts)"
