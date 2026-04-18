# Reviu Browser Extension

Open GitHub repositories, pull requests, and issues directly in the Reviu desktop app. Shared sources for the Chrome and Firefox extensions.

## Structure

- `src/` — shared content script, background script, styles, icons.
- `manifests/` — per-platform `manifest.json` (`chrome.json`, `firefox.json`).
- `scripts/build.sh` — assembles `dist/<target>/` and a zip per target.

## Dev

Build an unpacked directory first, then load it.

```sh
./scripts/build.sh chrome   # or: firefox
```

### Chrome

1. Open `chrome://extensions/`
2. Enable **Developer mode**
3. **Load unpacked** and select `extension/dist/chrome/`
4. Hit the refresh icon on the extension card after rebuilding

### Firefox

1. Open `about:debugging#/runtime/this-firefox`
2. **Load Temporary Add-on…** and select `extension/dist/firefox/manifest.json`
3. Click **Reload** on the extension card after rebuilding

## Publish

```sh
./scripts/build.sh            # builds both targets
./scripts/build.sh chrome     # one target only
```

Zips land in `dist/`:

- `reviu-chrome-extension-v<version>.zip` → [Chrome Web Store Developer Dashboard](https://chrome.google.com/webstore/devconsole)
- `reviu-firefox-extension-v<version>.zip` → [Firefox Add-on Developer Hub](https://addons.mozilla.org/developers/)
