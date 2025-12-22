import process from 'node:process'
import { z } from 'zod'
import 'dotenv/config'

const envSchema = z.object({
  NODE_ENV: z.enum(['development', 'production']),
  PG_USER: z.string(),
  PG_PASSWORD: z.string(),
  PG_HOST: z.string(),
  PG_PORT: z.coerce.number(),
  PG_DATABASE: z.string(),
  AUTH_SECRET: z.string(),
  GITHUB_OAUTH_CLIENT_SECRET: z.string(),
  GITHUB_OAUTH_CLIENT_ID: z.string(),
})

export const env = envSchema.parse(process.env)
