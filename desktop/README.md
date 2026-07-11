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

## Linux Packaging

Reviu includes an initial Linux packaging path based on a release tarball plus a user-level install script.

Build a Linux release archive:

```sh
bash ./desktop/scripts/build-linux-archive.sh 0.0.11 x86_64
```

This produces:

- `dist/release/linux/<target>/Reviu-x.y.z-linux-<arch>.tar.gz`
- `dist/release/linux/<target>/desktop-update.manifest.json`

Install the latest published Linux build for the current user:

```sh
bash ./desktop/scripts/install-linux.sh
```

The installer:

- downloads the latest Linux artifact from `desktop-update.manifest.json`
- installs Reviu under `~/.local/share/reviu`
- creates `~/.local/bin/reviu`
- writes a desktop entry to `~/.local/share/applications/reviu.desktop`
- registers the `reviu://` URL scheme when `xdg-mime` is available

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
