import { drizzle } from 'drizzle-orm/node-postgres'
import { env } from '../lib/env.js'
import * as schema from './schemas/index.js'

export const db = drizzle({
  schema,
  connection: {
    user: env.POSTGRES_USER,
    host: env.POSTGRES_HOST,
    database: env.POSTGRES_DB,
    password: env.POSTGRES_PASSWORD,
    port: env.POSTGRES_PORT,
  },
})
