import type { Context, Next } from 'hono'
import { createMiddleware } from 'hono/factory'
import { logger } from '../lib/logger.js'
import { withRedisClient } from '../lib/redis.js'

const REDIS_PREFIX = 'reviu:rl:'

interface RateLimitConfig {
  /** Max requests allowed in the window. */
  max: number
  /** Window duration in seconds. */
  windowSec: number
  /** Extract a key from the request (default: client IP). */
  keyFn?: (c: Context) => string | null
}

// In-memory fallback when Redis is unavailable.
const memoryStore = new Map<string, { count: number, expiresAt: number }>()

function memoryIncrement(key: string, windowSec: number): number {
  const now = Date.now()
  const entry = memoryStore.get(key)

  if (entry && entry.expiresAt > now) {
    entry.count++
    return entry.count
  }

  memoryStore.set(key, { count: 1, expiresAt: now + windowSec * 1000 })
  return 1
}

// Periodically clean expired entries so the map doesn't grow unbounded.
setInterval(() => {
  const now = Date.now()
  for (const [key, entry] of memoryStore) {
    if (entry.expiresAt <= now) {
      memoryStore.delete(key)
    }
  }
}, 60_000).unref()

async function redisIncrement(key: string, windowSec: number): Promise<number> {
  return withRedisClient('rate-limit', async (client) => {
    const fullKey = `${REDIS_PREFIX}${key}`
    const count = await client.incr(fullKey)
    if (count === 1) {
      await client.expire(fullKey, windowSec)
    }
    return count
  })
}

function getClientIp(c: Context): string {
  return (
    c.req.header('x-forwarded-for')?.split(',')[0]?.trim()
    || c.req.header('x-real-ip')
    || 'unknown'
  )
}

export function rateLimitMiddleware(config: RateLimitConfig) {
  const { max, windowSec, keyFn } = config

  return createMiddleware(async (c: Context, next: Next) => {
    const identifier = keyFn ? keyFn(c) : getClientIp(c)

    if (!identifier) {
      return next()
    }

    const key = `${c.req.path}:${identifier}:${Math.floor(Date.now() / (windowSec * 1000))}`

    let count: number
    try {
      count = await redisIncrement(key, windowSec)
    }
    catch {
      count = memoryIncrement(key, windowSec)
    }

    c.header('X-RateLimit-Limit', String(max))
    c.header('X-RateLimit-Remaining', String(Math.max(0, max - count)))

    if (count > max) {
      logger.warn({ key: `${c.req.path}:${identifier}`, count, max }, 'Rate limit exceeded')
      return c.json({ error: 'Too many requests' }, 429)
    }

    return next()
  })
}
