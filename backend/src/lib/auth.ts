import type { AsyncReturnType, Merge } from 'type-fest'
import { redisStorage } from '@better-auth/redis-storage'
import { checkout, polar, portal, webhooks } from '@polar-sh/better-auth'

import { betterAuth } from 'better-auth'
import { drizzleAdapter } from 'better-auth/adapters/drizzle'
import { admin, bearer, openAPI } from 'better-auth/plugins'

import { eq } from 'drizzle-orm'
import { db } from '../db/index.js'

import { user } from '../db/schemas/index.js'
import { env } from './env.js'
import { polarClient } from './polar.js'
import { withRedisClient } from './redis.js'
import { getTrustedOrigins } from './utils.js'

const BETTER_AUTH_REDIS_PREFIX = 'reviu:better-auth:'

async function withBetterAuthSecondaryStorage<T>(
  handler: (storage: ReturnType<typeof redisStorage>) => Promise<T>,
): Promise<T> {
  return withRedisClient('Better Auth secondary storage', async (client) => {
    return handler(redisStorage({
      client,
      keyPrefix: BETTER_AUTH_REDIS_PREFIX,
    }))
  })
}

export const auth = betterAuth({
  database: drizzleAdapter(db, {
    provider: 'pg',
  }),
  baseURL: env.BASE_URL,
  secondaryStorage: {
    get(key) {
      return withBetterAuthSecondaryStorage(storage => storage.get(key))
    },
    set(key, value, ttl) {
      return withBetterAuthSecondaryStorage(storage => storage.set(key, value, ttl))
    },
    delete(key) {
      return withBetterAuthSecondaryStorage(storage => storage.delete(key))
    },
  },
  trustedOrigins: getTrustedOrigins(),
  secret: env.AUTH_SECRET,
  socialProviders: {
    github: {
      clientId: env.GITHUB_OAUTH_CLIENT_ID,
      clientSecret: env.GITHUB_OAUTH_CLIENT_SECRET,
      scope: ['read:user', 'user:email', 'repo'],
    },
  },
  user: {
    additionalFields: {
      proGrantedAt: {
        type: 'date',
        input: false,
      },
      clientVersion: {
        type: 'string',
        input: false,
        required: false,
      },
      clientPlatform: {
        type: 'string',
        input: false,
        required: false,
      },
      clientArch: {
        type: 'string',
        input: false,
        required: false,
      },
      clientVersionUpdatedAt: {
        type: 'date',
        input: false,
        required: false,
      },
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
              productId: env.POLAR_SUBSCRIPTION_MONTHLY_PRODUCT_ID,
              slug: 'pro-monthly',
            },
            {
              productId: env.POLAR_SUBSCRIPTION_ANNUAL_PRODUCT_ID,
              slug: 'pro-annual',
            },
          ],
          successUrl: env.POLAR_SUCCESS_URL,
          authenticatedUsersOnly: true,
        }),
        portal(),
        webhooks({
          secret: env.POLAR_WEBHOOK_SECRET,
          onSubscriptionActive: async (payload) => {
            const { email } = payload.data.customer

            await db.update(user)
              .set({
                proGrantedAt: new Date(),
              })
              .where(eq(user.email, email))
          },
          onSubscriptionRevoked: async (payload) => {
            const { email } = payload.data.customer

            await db.update(user)
              .set({
                proGrantedAt: null,
              })
              .where(eq(user.email, email))
          },
        }),
      ],
    }),
  ],
})

export interface AuthType {
  user: (Merge<typeof auth.$Infer.Session.user, { role: 'user' | 'pro' | 'admin' }>) | null
  session: typeof auth.$Infer.Session.session | null
}

type AccessTokenType = AsyncReturnType<typeof auth.api.getAccessToken>

export type UserContext = Merge<NonNullable<AuthType['user']>, { github: AccessTokenType }>
