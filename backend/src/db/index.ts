import { drizzle } from 'drizzle-orm/node-postgres'
import { env } from '../lib/env.js'
import * as schema from './schemas/index.js'

export const db = drizzle({
  schema,
  connection: {
    user: env.PG_USER,
    host: env.PG_HOST,
    database: env.PG_DATABASE,
    password: env.PG_PASSWORD,
    port: env.PG_PORT,
  },
})
