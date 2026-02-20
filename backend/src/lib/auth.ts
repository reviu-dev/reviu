import type { AsyncReturnType, Merge } from 'type-fest'
import { checkout, polar, portal } from '@polar-sh/better-auth'

import { betterAuth } from 'better-auth'
import { drizzleAdapter } from 'better-auth/adapters/drizzle'
import { admin, bearer, openAPI } from 'better-auth/plugins'

import { db } from '../db/index.js'
import { env } from './env.js'

import { polarClient } from './polar.js'
import { getTrustedOrigins } from './utils.js'

export const auth = betterAuth({
  database: drizzleAdapter(db, {
    provider: 'pg',
  }),
  account: {
    skipStateCookieCheck: true,
  },
  baseURL: env.BASE_URL,
  trustedOrigins: getTrustedOrigins(),
  secret: env.AUTH_SECRET,
  socialProviders: {
    github: {
      clientId: env.GITHUB_OAUTH_CLIENT_ID,
      clientSecret: env.GITHUB_OAUTH_CLIENT_SECRET,
      scope: ['read:user', 'user:email', 'repo'],
    },
  },
  plugins: [
    bearer(),
    admin(),
    openAPI(),
    polar({
      client: polarClient,
      createCustomerOnSignUp: true,
      use: [
        checkout({
          products: [
            {
              productId: env.POLAR_SUBSCRIPTION_PRODUCT_ID,
              slug: 'pro',
            },
          ],
          successUrl: env.POLAR_SUCCESS_URL,
          authenticatedUsersOnly: true,
        }),
        portal(),
      ],
    }),
  ],
})

export interface AuthType {
  user: (Merge<typeof auth.$Infer.Session.user, { role: 'user' | 'admin' }>) | null
  session: typeof auth.$Infer.Session.session | null
}

type AccessTokenType = AsyncReturnType<typeof auth.api.getAccessToken>
type PolarStateType = AsyncReturnType<typeof auth.api.state>

export type UserContext = Merge<NonNullable<AuthType['user']>, { github: AccessTokenType, polar: PolarStateType }>
