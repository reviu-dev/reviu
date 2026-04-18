#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
EXT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SRC_DIR="$EXT_DIR/src"
MANIFESTS_DIR="$EXT_DIR/manifests"
OUT_DIR="$EXT_DIR/dist"

TARGETS=("$@")
if [ ${#TARGETS[@]} -eq 0 ]; then
  TARGETS=(chrome firefox)
fi

build_target() {
  local target="$1"
  local manifest="$MANIFESTS_DIR/$target.json"

  if [ ! -f "$manifest" ]; then
    echo "Unknown target: $target (no $manifest)" >&2
    exit 1
  fi

  local version
  version=$(grep '"version"' "$manifest" | head -1 | sed 's/.*: *"\(.*\)".*/\1/')

  local build_dir="$OUT_DIR/$target"
  local zip_name="reviu-$target-extension-v${version}.zip"

  rm -rf "$build_dir"
  mkdir -p "$build_dir"

  cp -R "$SRC_DIR/"* "$build_dir/"
  cp "$manifest" "$build_dir/manifest.json"

  (cd "$build_dir" && zip -r "$OUT_DIR/$zip_name" . >/dev/null)

  echo "Packaged: $OUT_DIR/$zip_name"
}

mkdir -p "$OUT_DIR"

for target in "${TARGETS[@]}"; do
  build_target "$target"
done
