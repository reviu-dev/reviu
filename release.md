# Release

## Desktop Release

The desktop release is currently manual because macOS builds are produced locally.
Linux and Windows artifacts are built by GitHub Actions from a release commit.

### 1. Trigger CI Builds

Create and push a release commit on `main`:

```bash
git commit -m "release: 0.4.0"
git push
```

The `Desktop Release` workflow builds:

- Linux `x86_64`
- Linux `aarch64`
- Windows `x86_64`
- Windows `aarch64`

Each CI artifact contains the platform package and a partial
`desktop-update.manifest.json`.

### 2. Build macOS Locally

Build the macOS DMG locally:

```bash
bash desktop/scripts/build-macos-dmg.sh 0.4.0 aarch64
```

If releasing an Intel macOS build too:

```bash
bash desktop/scripts/build-macos-dmg.sh 0.4.0 x86_64
```

The macOS build writes or updates the local manifest at:

```text
dist/release/desktop-update.manifest.json
```

### 3. Download CI Artifacts

Download and extract CI artifacts into separate folders. Do not extract all
partial manifests into the same directory without renaming them, because they
all use the same file name.

Suggested layout:

```text
dist/release/ci/linux-x86_64/desktop-update.manifest.json
dist/release/ci/linux-aarch64/desktop-update.manifest.json
dist/release/ci/windows-x86_64/desktop-update.manifest.json
dist/release/ci/windows-aarch64/desktop-update.manifest.json
```

### 4. Merge Manifests

Merge the local macOS manifest with the CI partial manifests:

```bash
bash desktop/scripts/merge-release-manifests.sh 0.10.0 \
  dist/release/desktop-update.manifest.json \
  dist/release/ci/linux-x86_64/desktop-update.manifest.json \
  dist/release/ci/linux-aarch64/desktop-update.manifest.json \
  dist/release/ci/windows-x86_64/desktop-update.manifest.json \
  dist/release/ci/windows-aarch64/desktop-update.manifest.json
```

The final manifest is written to:

```text
dist/release/desktop-update.manifest.json
```

### 5. Publish GitHub Release

Create the GitHub Release manually with tag `v0.4.0`.

Upload only the final release assets:

- macOS `.dmg` files
- Linux `.tar.gz` files
- Windows installer `.exe` files
- `dist/release/desktop-update.manifest.json`

Do not upload the partial CI manifests to the GitHub Release.
