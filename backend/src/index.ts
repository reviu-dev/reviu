import { serve } from '@hono/node-server'
import { app } from './app.js'
import { logger } from './lib/logger.js'
import './lib/env.js'

serve({
  fetch: app.fetch,
  port: 3000,
}, (info) => {
  logger.info(`Server is running on ${info.port}`)
})
