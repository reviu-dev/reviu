import process from 'node:process'
import { z } from 'zod'
import 'dotenv/config'

const envSchema = z.object({
  NODE_ENV: z.enum(['development', 'production']),
  BASE_URL: z.string(),
  PG_USER: z.string(),
  PG_PASSWORD: z.string(),
  PG_HOST: z.string(),
  PG_PORT: z.coerce.number(),
  PG_DATABASE: z.string(),
  AUTH_SECRET: z.string(),
  GITHUB_OAUTH_CLIENT_SECRET: z.string(),
  GITHUB_OAUTH_CLIENT_ID: z.string(),
  POLAR_ACCESS_TOKEN: z.string(),
  POLAR_SUCCESS_URL: z.string(),
  POLAR_SUBSCRIPTION_PRODUCT_ID: z.string(),
})

export const env = envSchema.parse(process.env)
