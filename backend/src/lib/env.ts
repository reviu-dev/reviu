import process from 'node:process'
import { z } from 'zod'
import 'dotenv/config'

const envSchema = z.object({
  NODE_ENV: z.enum(['development', 'production']),
  BASE_URL: z.string(),
  PORT: z.coerce.number().int().positive(),
  POSTGRES_USER: z.string(),
  POSTGRES_PASSWORD: z.string(),
  POSTGRES_HOST: z.string(),
  POSTGRES_PORT: z.coerce.number(),
  POSTGRES_DB: z.string(),
  AUTH_SECRET: z.string(),
  GITHUB_OAUTH_CLIENT_SECRET: z.string(),
  GITHUB_OAUTH_CLIENT_ID: z.string(),
  GITHUB_TOKEN: z.string(),
  REDIS_HOST: z.string(),
  REDIS_PORT: z.coerce.number(),
  REDIS_PASSWORD: z.string(),
  LOG_GITHUB_CACHE: z.stringbool().default(false),
  GITHUB_CACHE_ENABLED: z.stringbool().default(true),
  GITHUB_METRICS_FLUSH_INTERVAL_MS: z.coerce.number().positive(),
  GITHUB_METRICS_RETENTION_DAYS: z.coerce.number().int().positive(),
  GITHUB_RATE_LIMIT_STATE_RETENTION_DAYS: z.coerce.number().int().positive(),
  CLIENT_ANALYTICS_RETENTION_DAYS: z.coerce.number().int().positive(),
  POLAR_ACCESS_TOKEN: z.string(),
  POLAR_SUCCESS_URL: z.url(),
  POLAR_SUBSCRIPTION_PRODUCT_ID: z.string(),
  POLAR_WEBHOOK_SECRET: z.string(),
  WEB_DASHBOARD_URL: z.url(),
  SHIPIT_API_KEY: z.string(),
  SHIPIT_PROJECT_ID: z.string(),
  SHIPIT_API_URL: z.url(),
})

export type Env = z.infer<typeof envSchema>

export const env = envSchema.parse(process.env)
