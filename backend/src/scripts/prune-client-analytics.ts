import process from 'node:process'
import { db } from '../db/index.js'
import { logger } from '../lib/logger.js'
import { pruneClientAnalytics } from '../plugins/client-analytics/client-analytics-store.js'
import '../lib/env.js'

async function closeDatabaseClient() {
  await db.$client.end()
}

async function main() {
  try {
    const result = await pruneClientAnalytics()

    logger.info({
      retentionDays: result.retentionDays,
      cutoff: result.cutoff.toISOString(),
      deletedRows: result.deletedRows,
    }, 'Pruned client analytics from Postgres')
  }
  catch (error) {
    logger.error({ error }, 'Failed to prune client analytics from Postgres')
    process.exitCode = 1
  }
  finally {
    await closeDatabaseClient()
  }
}

await main()
