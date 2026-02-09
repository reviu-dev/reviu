import process from 'node:process'
import { serve } from '@hono/node-server'
import { app } from './app.js'
import { logger } from './lib/logger.js'
import './lib/env.js'

const server = serve({
  fetch: app.fetch,
  port: 3000,
}, (info) => {
  logger.info(`Server is running on ${info.port}`)
})

process.on('SIGINT', () => {
  server.close()
  process.exit(0)
})
process.on('SIGTERM', () => {
  server.close((err) => {
    if (err) {
      console.error(err)
      process.exit(1)
    }
    process.exit(0)
  })
})
