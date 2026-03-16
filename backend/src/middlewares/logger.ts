import { createMiddleware } from 'hono/factory'
import { logger } from '../lib/logger.js'

const IGNORED_PATHS = ['/healthcheck']

export const loggerMiddleware = createMiddleware(async (c, next) => {
  const start = Date.now()

  await next()

  const method = c.req.method
  const url = new URL(c.req.url)
  const durationMs = Date.now() - start
  const status = c.res.status

  if (IGNORED_PATHS.includes(url.pathname)) {
    return
  }

  const log = `${method} ${status} ${url.pathname}${url.search} - ${durationMs}ms`

  logger.info(log)
})
