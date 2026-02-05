import { createMiddleware } from 'hono/factory'
import { logger } from '../lib/logger.js'

export const loggerMiddleware = createMiddleware(async (c, next) => {
  const start = Date.now()

  await next()

  const method = c.req.method
  const pathname = new URL(c.req.url).pathname
  const durationMs = Date.now() - start

  const log = `${method} ${pathname} - ${durationMs}ms`

  logger.info(log)
})
