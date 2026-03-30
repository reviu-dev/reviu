# Desktop

## Setup

```
docker compose -p reviu up -d
```

```
pnpm install
pnpm dev
```

## Better auth swagger

```
open http://localhost:3000/api/auth/reference
```

## Polar webhooks

Install CLI: https://polar.sh/docs/integrate/webhooks/locally#install-polar-cli

```sh
polar listen http://localhost:3000/
```

Set POLAR_WEBHOOK_SECRET
