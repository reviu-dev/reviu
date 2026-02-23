# Reviu

Desktop Git client with a keyboard-first workflow.  
`Free`: local Git. `Reviu Pro`: GitHub integration.

## Monorepo

- `desktop/`: Rust + GPUI app
- `backend/`: Hono API + Better Auth + Polar billing
- `landing/`: Astro + Vue marketing site

## Run locally

### Desktop

```sh
cd desktop && cargo run
```

#### With Sentry

```sh
cd desktop && SENTRY_ENABLE_DEV=1 cargo run
```

```sh
cd desktop && cargo run --release -p reviu
```

`SENTRY_ENABLE_DEV` is only read in debug builds

### Backend

Requirements: Node and pnpm

```sh
cd backend
nvm use
pnpm install
pnpm dev
```

Required env vars are validated in `backend/src/lib/env.ts`.

### Landing

```sh
cd landing
pnpm install
pnpm dev
```

## Useful docs

- [Product/features summary](APP_FEATURES.md)
- [Agent/dev map](AGENTS.md)
- [Desktop build/release details](desktop/README.md)
