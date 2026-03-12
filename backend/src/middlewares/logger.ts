import { createMiddleware } from 'hono/factory'
import { logger } from '../lib/logger.js'

export const loggerMiddleware = createMiddleware(async (c, next) => {
  const start = Date.now()

  await next()

  const method = c.req.method
  const url = new URL(c.req.url)
  const durationMs = Date.now() - start
  const status = c.res.status

  const log = `${method} ${status} ${url.pathname}${url.search} - ${durationMs}ms`

  logger.info(log)
})
