# Reviu Chrome Extension

Open GitHub repositories, pull requests, and issues directly in the Reviu desktop app.

## Dev

1. Open `chrome://extensions/`
2. Enable **Developer mode** (top right)
3. Click **Load unpacked** and select the `chrome-extension/` folder
4. Navigate to any GitHub repo, PR, or issue, the button and toolbar icon should appear

After code changes, hit the refresh icon on the extension card in `chrome://extensions/`.

## Publish

```sh
./scripts/package.sh
```

This creates a zip in `dist/` ready to upload on the [Chrome Web Store Developer Dashboard](https://chrome.google.com/webstore/devconsole).
