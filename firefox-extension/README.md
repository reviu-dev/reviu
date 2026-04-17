# Reviu Firefox Extension

Open GitHub repositories, pull requests, and issues directly in the Reviu desktop app.

## Dev

1. Open `about:debugging#/runtime/this-firefox`
2. Click **Load Temporary Add-on…**
3. Select the `manifest.json` file inside the `firefox-extension/` folder
4. Navigate to any GitHub repo, PR, or issue, the button and toolbar icon should appear

After code changes, click **Reload** on the extension card in `about:debugging`.

## Publish

```sh
./scripts/package.sh
```

This creates a zip in `dist/` ready to upload on the [Firefox Add-on Developer Hub](https://addons.mozilla.org/developers/).
