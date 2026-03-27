import type { GithubCacheStore } from '../plugins/github/cache/github-cache.js'

import { Redis } from 'ioredis'
import { MemoryGithubCacheStore } from '../plugins/github/cache/github-cache.js'
import { env } from './env.js'
import { logger } from './logger.js'

const RELEASE_LOCK_SCRIPT = `
if redis.call("get", KEYS[1]) == ARGV[1] then
  return redis.call("del", KEYS[1])
end
return 0
`

let redisClient: Redis | null = null
let redisConnectPromise: Promise<Redis | null> | null = null

function waitForReady(client: Redis): Promise<Redis> {
  if (client.status === 'ready') {
    return Promise.resolve(client)
  }

  return new Promise((resolve, reject) => {
    const onReady = () => {
      cleanup()
      resolve(client)
    }
    const onError = (error: Error) => {
      cleanup()
      reject(error)
    }
    const onEnd = () => {
      cleanup()
      reject(new Error('Redis connection ended before becoming ready'))
    }

    function cleanup() {
      client.off('ready', onReady)
      client.off('error', onError)
      client.off('end', onEnd)
    }

    client.on('ready', onReady)
    client.on('error', onError)
    client.on('end', onEnd)
  })
}

async function getRedisClient(): Promise<Redis | null> {
  if (!redisClient) {
    redisClient = new Redis(env.REDIS_PORT, env.REDIS_HOST, {
      lazyConnect: true,
      maxRetriesPerRequest: 1,
      password: env.REDIS_PASSWORD,
      enableOfflineQueue: false,
      retryStrategy(times: number) {
        return Math.min(times * 50, 2_000)
      },
    })

    redisClient.on('error', (error: Error) => {
      logger.warn({ error }, 'Redis client error')
    })
  }

  if (redisClient.status === 'ready') {
    return redisClient
  }

  if (!redisConnectPromise) {
    redisConnectPromise = (async () => {
      try {
        if (redisClient!.status === 'wait') {
          await redisClient!.connect()
        }
        else {
          await waitForReady(redisClient!)
        }

        return redisClient
      }
      catch (error) {
        logger.warn({ error }, 'Failed to connect to Redis')
        return null
      }
      finally {
        redisConnectPromise = null
      }
    })()
  }

  return redisConnectPromise
}

export async function withRedisClient<T>(
  context: string,
  handler: (client: Redis) => Promise<T>,
): Promise<T> {
  const client = await getRedisClient()

  if (!client) {
    throw new Error(`${context} unavailable because Redis could not be reached`)
  }

  return handler(client)
}

export async function assertRedisHealthy(): Promise<void> {
  const client = await getRedisClient()

  if (!client) {
    throw new Error('Redis unavailable, using in-memory fallback cache')
  }

  const result = await client.ping()

  if (result !== 'PONG') {
    throw new Error(`Unexpected Redis ping response: ${result}`)
  }
}

class RedisGithubCacheStore implements GithubCacheStore {
  private readonly fallback = new MemoryGithubCacheStore()

  async get(key: string): Promise<string | null> {
    const client = await getRedisClient()
    if (!client) {
      return this.fallback.get(key)
    }

    try {
      return await client.get(key)
    }
    catch (error) {
      logger.warn({ error, key }, 'Redis GET failed, using fallback cache')
      return this.fallback.get(key)
    }
  }

  async set(key: string, value: string): Promise<void> {
    await this.fallback.set(key, value)

    const client = await getRedisClient()
    if (!client) {
      return
    }

    try {
      await client.set(key, value)
    }
    catch (error) {
      logger.warn({ error, key }, 'Redis SET failed, using fallback cache')
    }
  }

  async del(keys: string[]): Promise<void> {
    if (keys.length === 0) {
      return
    }

    await this.fallback.del(keys)

    const client = await getRedisClient()
    if (!client) {
      return
    }

    try {
      await client.del(keys)
    }
    catch (error) {
      logger.warn({ error, keys }, 'Redis DEL failed, using fallback cache')
    }
  }

  async addToSet(key: string, members: string[]): Promise<void> {
    await this.fallback.addToSet(key, members)

    if (members.length === 0) {
      return
    }

    const client = await getRedisClient()
    if (!client) {
      return
    }

    try {
      await client.sadd(key, ...members)
    }
    catch (error) {
      logger.warn({ error, key }, 'Redis SADD failed, using fallback cache')
    }
  }

  async removeFromSet(key: string, members: string[]): Promise<void> {
    await this.fallback.removeFromSet(key, members)

    if (members.length === 0) {
      return
    }

    const client = await getRedisClient()
    if (!client) {
      return
    }

    try {
      await client.srem(key, ...members)
    }
    catch (error) {
      logger.warn({ error, key }, 'Redis SREM failed, using fallback cache')
    }
  }

  async getSetMembers(key: string): Promise<string[]> {
    const client = await getRedisClient()
    if (!client) {
      return this.fallback.getSetMembers(key)
    }

    try {
      return await client.smembers(key)
    }
    catch (error) {
      logger.warn({ error, key }, 'Redis SMEMBERS failed, using fallback cache')
      return this.fallback.getSetMembers(key)
    }
  }

  async setIfNotExists(key: string, value: string, ttlMs: number): Promise<boolean> {
    const fallbackAcquired = await this.fallback.setIfNotExists(key, value, ttlMs)
    const client = await getRedisClient()

    if (!client) {
      return fallbackAcquired
    }

    try {
      const result = await client.set(key, value, 'PX', ttlMs, 'NX')
      return result === 'OK'
    }
    catch (error) {
      logger.warn({ error, key }, 'Redis lock acquisition failed, using fallback cache')
      return fallbackAcquired
    }
  }

  async releaseLock(key: string, value: string): Promise<void> {
    await this.fallback.releaseLock(key, value)

    const client = await getRedisClient()
    if (!client) {
      return
    }

    try {
      await client.eval(RELEASE_LOCK_SCRIPT, 1, key, value)
    }
    catch (error) {
      logger.warn({ error, key }, 'Redis lock release failed, using fallback cache')
    }
  }
}

const defaultGithubCacheStore = new RedisGithubCacheStore()

export function createDefaultGithubCacheStore(): GithubCacheStore {
  return defaultGithubCacheStore
}
