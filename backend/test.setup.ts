import type { Stringified } from 'type-fest'
import type { Env } from './src/lib/env.js'
import process from 'node:process'

function setupEnv() {
  const env: Stringified<Env> = {
    NODE_ENV: 'development',
    PORT: '4000',
    POSTGRES_USER: 'user',
    POSTGRES_PASSWORD: 'password',
    POSTGRES_HOST: 'localhost',
    POSTGRES_PORT: '5433',
    POSTGRES_DB: 'app',
    AUTH_SECRET: 'auth-secret',
    BASE_URL: 'http://localhost:4000',
    GITHUB_METRICS_FLUSH_INTERVAL_MS: '60000',
    GITHUB_METRICS_RETENTION_DAYS: '30',
    GITHUB_RATE_LIMIT_STATE_RETENTION_DAYS: '14',
    GITHUB_OAUTH_CLIENT_ID: 'github-client-id',
    GITHUB_OAUTH_CLIENT_SECRET: 'github-client-secret',
    REDIS_HOST: 'localhost',
    REDIS_PORT: '6379',
    REDIS_PASSWORD: 'redis-password',
    GITHUB_CACHE_ENABLED: 'true',
    GITHUB_TOKEN: 'github-token',
    POLAR_ACCESS_TOKEN: 'polar-token',
    POLAR_SUCCESS_URL: 'http://localhost:3000/polar/success',
    POLAR_SUBSCRIPTION_PRODUCT_ID: 'product-id',
    POLAR_WEBHOOK_SECRET: 'polar-webhook-secret',
    WEB_DASHBOARD_URL: 'http://localhost:5173',
    SHIPIT_API_KEY: 'shipit-api-key',
    SHIPIT_PROJECT_ID: 'shipit-project-id',
  }

  for (const [key, value] of Object.entries(env)) {
    process.env[key] = value
  }
}

setupEnv()
