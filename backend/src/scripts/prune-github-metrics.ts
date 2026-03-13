import process from 'node:process'
import { db } from '../db/index.js'
import { logger } from '../lib/logger.js'
import { pruneGithubMetrics } from '../plugins/github/metrics/github-metrics-store.js'
import '../lib/env.js'

async function closeDatabaseClient() {
  await db.$client.end()
}

async function main() {
  try {
    const result = await pruneGithubMetrics()

    logger.info({
      metricsRetentionDays: result.metricsRetentionDays,
      rateLimitStateRetentionDays: result.rateLimitStateRetentionDays,
      metricsCutoff: result.metricsCutoff.toISOString(),
      rateLimitStateCutoff: result.rateLimitStateCutoff.toISOString(),
      deletedOperationMetrics: result.deletedOperationMetrics,
      deletedResourceMetrics: result.deletedResourceMetrics,
      deletedUserMetrics: result.deletedUserMetrics,
      deletedRateLimitStates: result.deletedRateLimitStates,
      totalDeleted: result.totalDeleted,
    }, 'Pruned GitHub metrics from Postgres')
  }
  catch (error) {
    logger.error({ error }, 'Failed to prune GitHub metrics from Postgres')
    process.exitCode = 1
  }
  finally {
    await closeDatabaseClient()
  }
}

await main()
