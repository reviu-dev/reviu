FROM node:24-alpine AS base

FROM base AS builder

RUN apk add --no-cache gcompat
WORKDIR /app

COPY package.json pnpm-lock.yaml tsconfig.json src ./

RUN npm i -g corepack@latest && corepack enable

RUN pnpm install --frozen-lockfile && \
    pnpm build && \
    pnpm prune --production

FROM base AS runner
WORKDIR /app

RUN addgroup --system --gid 1001 nodejs
RUN adduser --system --uid 1001 hono

COPY --from=builder --chown=hono:nodejs /app/node_modules /app/node_modules
COPY --from=builder --chown=hono:nodejs /app/dist /app/dist
COPY --from=builder --chown=hono:nodejs /app/package.json /app/package.json

ENV NODE_ENV=production
ENV PORT=4000

USER hono
EXPOSE 4000

CMD ["node", "/app/dist/index.js"]