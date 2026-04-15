#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
EXT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
VERSION=$(grep '"version"' "$EXT_DIR/manifest.json" | head -1 | sed 's/.*: *"\(.*\)".*/\1/')
OUT_DIR="$EXT_DIR/dist"
ZIP_NAME="reviu-chrome-extension-v${VERSION}.zip"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

cd "$EXT_DIR"
zip -r "$OUT_DIR/$ZIP_NAME" \
  manifest.json \
  background.js \
  content.js \
  content.css \
  icons/

echo ""
echo "Packaged: $OUT_DIR/$ZIP_NAME"
echo "Upload it at: https://chrome.google.com/webstore/devconsole"
