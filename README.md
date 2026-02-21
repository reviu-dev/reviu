# Reviu

## Desktop with Sentry

Dev (Sentry OFF by default):

```sh
cd desktop && cargo run
```

Dev (Sentry ON for tests):

```sh
cd desktop && SENTRY_ENABLE_DEV=1 cargo run
```

Release (Sentry ON):

```sh
cd desktop && cargo run --release -p reviu
```

`SENTRY_ENABLE_DEV` is only read in debug builds. In release builds, Sentry stays enabled.
