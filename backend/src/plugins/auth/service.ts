import crypto from 'node:crypto'
import { withRedisClient } from '../../lib/redis.js'

export const AUTH_CODE_TTL_MS = 5 * 60 * 1000 // 5 minutes
const AUTH_CODE_KEY_PREFIX = 'reviu:auth-code:'

export interface AuthCodeStore {
  set: (key: string, value: string, ttlMs: number) => Promise<void>
  getDel: (key: string) => Promise<string | null>
}

function buildAuthCodeKey(code: string) {
  return `${AUTH_CODE_KEY_PREFIX}${code}`
}

class RedisAuthCodeStore implements AuthCodeStore {
  async set(key: string, value: string, ttlMs: number): Promise<void> {
    await withRedisClient('Auth code store', async (client) => {
      await client.set(key, value, 'PX', ttlMs)
    })
  }

  async getDel(key: string): Promise<string | null> {
    return withRedisClient('Auth code store', async (client) => {
      const value = await client.call('GETDEL', key)

      if (typeof value !== 'string') {
        return null
      }

      return value
    })
  }
}

export function createAuthCodeService(store: AuthCodeStore) {
  return {
    async issueAuthCode(token: string) {
      const code = crypto.randomBytes(32).toString('hex')
      await store.set(buildAuthCodeKey(code), token, AUTH_CODE_TTL_MS)
      return code
    },
    async consumeAuthCode(code: string) {
      return store.getDel(buildAuthCodeKey(code))
    },
  }
}

const authCodeService = createAuthCodeService(new RedisAuthCodeStore())

export const issueAuthCode = authCodeService.issueAuthCode
export const consumeAuthCode = authCodeService.consumeAuthCode
