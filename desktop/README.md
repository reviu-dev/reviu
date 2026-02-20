# Desktop

## Build

```
cargo bundle -p reviu --release
open target/release/bundle/osx/Reviu.app
```

## Release CD

Desktop release publication is automated by `/.github/workflows/desktop-release.yml`.

Rules:

- Runs only after `CI` succeeds.
- Runs only for commits on `main`.
- Runs only when the commit subject exactly matches: `release: x.y.z`.
- Builds macOS arm64, creates `Reviu-x.y.z-macos-aarch64.dmg`, and publishes it to GitHub Releases (`vx.y.z`).
- Generates and uploads `desktop-update.manifest.json` to the same release.

Recommended backend production env:

```
DESKTOP_UPDATE_MANIFEST_URL=https://github.com/<owner>/<repo>/releases/latest/download/desktop-update.manifest.json
```
