FROM node:26-alpine AS build

WORKDIR /app

# pnpm runs its pre-script deps check non-interactively; without this it aborts on no TTY.
ENV CI=true

RUN npm i -g corepack@latest && corepack enable

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./

RUN pnpm install --frozen-lockfile

COPY . .

ENV PUBLIC_BACKEND_URL=https://api.reviu.dev

RUN pnpm build

FROM nginx:alpine AS runtime

COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html

EXPOSE 80
