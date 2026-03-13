import { defineConfig } from 'drizzle-kit'
// @ts-expect-error .js error import
import { env } from './src/lib/env'
import 'dotenv/config'

const pgUrl = `postgres://${env.PG_USER}:${env.PG_PASSWORD}@${env.PG_HOST}:${env.PG_PORT}/${env.PG_DATABASE}`

export default defineConfig({
  out: './drizzle',
  schema: './src/db/schemas/index.ts',
  dialect: 'postgresql',
  dbCredentials: {
    url: pgUrl,
  },
})
