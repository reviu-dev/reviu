import type { Merge } from 'type-fest'
import { betterAuth } from 'better-auth'
import { drizzleAdapter } from 'better-auth/adapters/drizzle'
import { admin, openAPI } from 'better-auth/plugins'
import { db } from '../db/index.js'
import { env } from './env.js'

import { getTrustedOrigins } from './utils.js'

export const auth = betterAuth({
  database: drizzleAdapter(db, {
    provider: 'pg',
  }),
  baseURL: env.BASE_URL,
  trustedOrigins: getTrustedOrigins(),
  secret: env.AUTH_SECRET,
  socialProviders: {
    github: {
      clientId: env.GITHUB_OAUTH_CLIENT_ID,
      clientSecret: env.GITHUB_OAUTH_CLIENT_SECRET,
    },
  },
  plugins: [
    admin(),
    openAPI(),
  ],
})

export interface AuthType {
  user: (Merge<typeof auth.$Infer.Session.user, { role: 'user' | 'admin' }>) | null
  session: typeof auth.$Infer.Session.session | null
}
