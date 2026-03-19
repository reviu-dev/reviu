# Desktop

## Build

```
cargo bundle -p reviu --release
open target/release/bundle/osx/Reviu.app
```

For a production bundle, inject the backend URL at compile time:

```sh
API_BASE_URL=https://api.reviu.dev cargo bundle -p reviu --release
```

## Profiles

Desktop supports two local profiles:

- `prod`: the normal app profile
- `dev`: an isolated development profile

Default behavior:

- debug builds run as `dev`
- release builds run as `prod`

You can override the profile explicitly with `REVIU_PROFILE`.

Examples:

```sh
cargo run -p reviu
```

```sh
REVIU_PROFILE=dev cargo run -p reviu
```

```sh
REVIU_PROFILE=prod cargo run -p reviu
```

What changes per profile:

- keychain service: `reviu_auth` / `reviu_auth.dev`
- local config and app data directories: `reviu` / `reviu.dev`
- desktop deep link scheme: `reviu://` / `reviu-dev://`
- app header badge: no badge in prod, `DEV` in dev

This lets you run the installed app and the local dev build side by side without sharing auth state, settings, recent repositories, or downloaded updates.

## Release CD

Desktop release publication is automated by [`/.github/workflows/desktop-release.yml`](../.github/workflows/desktop-release.yml).

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
