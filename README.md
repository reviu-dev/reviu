# Reviu

A keyboard-first desktop Git client, built in Rust with [GPUI](https://gpui.rs).
Review your AI agent's code before you push, then take it to merge.

- **Free**: local Git workflow + a built-in agent panel (Claude and Codex).
- **Reviu Pro**: in-app GitHub integration (notifications, pull request review,
  issues, merge actions). `$9/month` or `$79/year`.

Download builds at [reviu.dev](https://reviu.dev).

## Repository layout

- `desktop/`: the Rust + GPUI desktop app (this is the client).
- `landing/`: the Astro marketing site ([reviu.dev](https://reviu.dev)).
- `extension/`: browser extension for GitHub repos, PRs, and issues.

The GitHub-integration backend (the service powering Reviu Pro) is closed-source
and lives in a separate private repository. The Free features (local Git and the
agent panel) run fully without it.

## Build from source

### Desktop

```sh
cd desktop && cargo run
```

Release build:

```sh
cd desktop && cargo run --release -p reviu
```

With Sentry enabled in a debug build (off by default; `SENTRY_ENABLE_DEV` is only
read in debug builds):

```sh
cd desktop && SENTRY_ENABLE_DEV=1 cargo run
```

See [`desktop/README.md`](desktop/README.md) for build/release details.

## License

Source-available under [FSL-1.1-ALv2](LICENSE) (Functional Source License, Apache 2.0
future license): use, modify and redistribute freely for any non-competing purpose;
each version converts to Apache-2.0 two years after its release.

## Security

See [SECURITY.md](SECURITY.md) to report a vulnerability.
