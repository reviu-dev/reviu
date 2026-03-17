# Build stage
FROM node:24-alpine AS build

WORKDIR /app

COPY package.json pnpm-lock.yaml .

RUN npm i -g corepack@latest && corepack enable
RUN pnpm install --frozen-lockfile

COPY . .

ENV VITE_BACKEND_URL=https://api.reviu.dev

RUN pnpm build

# Serve stage
FROM nginx:alpine

COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html

EXPOSE 80