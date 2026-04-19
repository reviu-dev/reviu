import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { db } from '../db/index.js'
import { logger } from '../lib/logger.js'
import { authMiddlewareAdmin } from '../middlewares/auth.js'
import {
  flushGithubMetricsNow,
  readGithubMetricsOperationDrilldownFromDatabase,
  readGithubMetricsOverviewFromDatabase,
} from '../plugins/github/metrics/github-metrics-store.js'
import { githubMetricsCollector } from '../plugins/github/metrics/github-metrics.js'

const adminRouter = new Hono()

const overviewQuerySchema = z.object({
  windowMinutes: z.coerce.number().int().min(1).max(24 * 60).default(60),
  limit: z.coerce.number().int().min(1).max(50).default(10),
})

const drilldownQuerySchema = z.object({
  windowMinutes: z.coerce.number().int().min(1).max(24 * 60).default(60),
  operation: z.string().min(1),
  scope: z.enum(['viewer', 'public', 'installation']).optional(),
})

async function getUsersById(userIds: string[]) {
  if (userIds.length === 0) {
    return new Map<string, { id: string, name: string, email: string }>()
  }

  const rows = await db.query.user.findMany({
    columns: {
      id: true,
      name: true,
      email: true,
    },
    where: (user, { inArray }) => inArray(user.id, userIds),
  })

  return new Map(rows.map(row => [row.id, row]))
}

adminRouter.use('*', authMiddlewareAdmin)

export const adminRoutes = adminRouter
  .get(
    '/github-cache/overview',
    zValidator('query', overviewQuerySchema),
    async (ctx) => {
      const { windowMinutes, limit } = ctx.req.valid('query')
      const windowMs = windowMinutes * 60_000
      let overview

      try {
        await flushGithubMetricsNow()
        overview = await readGithubMetricsOverviewFromDatabase({
          windowMs,
          limit,
        })
      }
      catch (error) {
        logger.warn({ error }, 'Falling back to in-memory GitHub metrics overview')
        overview = githubMetricsCollector.getOverview({
          windowMs,
          limit,
        })
      }

      const userIds = [...new Set([
        ...overview.users.map(item => item.userId),
        ...overview.currentRateLimits.map(item => item.userId),
      ])]

      const usersById = await getUsersById(userIds)

      return ctx.json({
        ...overview,
        users: overview.users.map(item => ({
          ...item,
          user: usersById.get(item.userId) ?? null,
        })),
        currentRateLimits: overview.currentRateLimits.map(item => ({
          ...item,
          user: usersById.get(item.userId) ?? null,
        })),
      }, 200)
    },
  )
  .get(
    '/github-cache/drilldown',
    zValidator('query', drilldownQuerySchema),
    async (ctx) => {
      const { windowMinutes, operation, scope } = ctx.req.valid('query')
      const windowMs = windowMinutes * 60_000
      let drilldown

      try {
        await flushGithubMetricsNow()
        drilldown = await readGithubMetricsOperationDrilldownFromDatabase({
          windowMs,
          operation,
          scope,
        })
      }
      catch (error) {
        logger.warn({ error, operation, scope }, 'Falling back to in-memory GitHub metrics drilldown')
        drilldown = githubMetricsCollector.getOperationDrilldown({
          windowMs,
          operation,
          scope,
        })
      }

      return ctx.json(drilldown, 200)
    },
  )
