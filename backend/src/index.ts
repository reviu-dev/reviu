import process from 'node:process'
import { serve } from '@hono/node-server'
import { app } from './app.js'
import { env } from './lib/env.js'
import { logger } from './lib/logger.js'
import { flushGithubMetricsNow, startGithubMetricsPersistence, stopGithubMetricsPersistence } from './plugins/github/metrics/github-metrics-store.js'

const server = serve({
  fetch: app.fetch,
  port: env.PORT,
}, (info) => {
  logger.info(`Server is running on ${info.port}`)
})

startGithubMetricsPersistence()

let shuttingDown = false

function closeServer() {
  return new Promise<void>((resolve, reject) => {
    server.close((error) => {
      if (error) {
        reject(error)
        return
      }

      resolve()
    })
  })
}

async function shutdown(exitCode: number) {
  if (shuttingDown) {
    return
  }

  shuttingDown = true
  stopGithubMetricsPersistence()

  try {
    await closeServer()
    await flushGithubMetricsNow()
  }
  catch (error) {
    logger.error({ error }, 'Failed to shutdown server cleanly')
    process.exit(1)
  }

  process.exit(exitCode)
}

process.on('SIGINT', () => {
  void shutdown(0)
})
process.on('SIGTERM', () => {
  void shutdown(0)
})
