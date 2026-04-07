import type { ClientRoutePersistedMetric } from './client-analytics-collector.js'
import { and, gte, lte, sql } from 'drizzle-orm'
import { db } from '../../db/index.js'
import { clientRouteMetricMinute } from '../../db/schemas/index.js'
import { env } from '../../lib/env.js'
import { logger } from '../../lib/logger.js'
import { clientAnalyticsCollector } from './client-analytics-collector.js'

const FLUSH_INTERVAL_MS = 60_000
const DEFAULT_WINDOW_MINUTES = 1440

function excludedColumn<T extends { name: string }>(column: T) {
  return sql.raw(`excluded.${column.name}`)
}

async function persistMetrics(metrics: ClientRoutePersistedMetric[]) {
  if (metrics.length === 0) {
    return
  }

  await db
    .insert(clientRouteMetricMinute)
    .values(metrics.map(m => ({
      bucketStart: new Date(m.bucketStart),
      clientVersion: m.clientVersion,
      clientPlatform: m.clientPlatform,
      clientArch: m.clientArch,
      method: m.method,
      route: m.route,
      requests: m.requests,
      uniqueUserIds: m.uniqueUserIds,
      lastSeenAt: new Date(m.lastSeenAt),
    })))
    .onConflictDoUpdate({
      target: [
        clientRouteMetricMinute.bucketStart,
        clientRouteMetricMinute.clientVersion,
        clientRouteMetricMinute.method,
        clientRouteMetricMinute.route,
      ],
      set: {
        requests: sql`${clientRouteMetricMinute.requests} + ${excludedColumn(clientRouteMetricMinute.requests)}`,
        uniqueUserIds: sql`(
          select coalesce(array_agg(distinct uid), '{}')
          from unnest(${clientRouteMetricMinute.uniqueUserIds} || ${excludedColumn(clientRouteMetricMinute.uniqueUserIds)}) as uid
          where uid is not null
        )`,
        lastSeenAt: sql`greatest(${clientRouteMetricMinute.lastSeenAt}, ${excludedColumn(clientRouteMetricMinute.lastSeenAt)})`,
        clientPlatform: sql`coalesce(${excludedColumn(clientRouteMetricMinute.clientPlatform)}, ${clientRouteMetricMinute.clientPlatform})`,
        clientArch: sql`coalesce(${excludedColumn(clientRouteMetricMinute.clientArch)}, ${clientRouteMetricMinute.clientArch})`,
      },
    })
}

let flushInterval: NodeJS.Timeout | null = null
let flushPromise: Promise<void> | null = null

export async function flushClientAnalyticsNow() {
  if (flushPromise) {
    return flushPromise
  }

  flushPromise = (async () => {
    const { metrics } = clientAnalyticsCollector.drainPersistedMetrics()
    if (metrics.length === 0) {
      return
    }

    try {
      await persistMetrics(metrics)
    }
    catch (error) {
      clientAnalyticsCollector.requeuePersistedMetrics(metrics)
      throw error
    }
  })()
    .catch((error) => {
      logger.warn({ error }, 'Failed to flush client analytics to Postgres')
    })
    .finally(() => {
      flushPromise = null
    })

  return flushPromise
}

export function startClientAnalyticsPersistence() {
  if (flushInterval) {
    return
  }

  flushInterval = setInterval(() => {
    void flushClientAnalyticsNow()
    clientAnalyticsCollector.prune()
  }, FLUSH_INTERVAL_MS)

  flushInterval.unref?.()
}

export function stopClientAnalyticsPersistence() {
  if (!flushInterval) {
    return
  }

  clearInterval(flushInterval)
  flushInterval = null
}

export interface ClientAnalyticsVersionSummary {
  clientVersion: string
  clientPlatform: string | null
  clientArch: string | null
  requests: number
  uniqueUsers: number
  lastSeenAt: number
}

export interface ClientAnalyticsRouteSummary {
  method: string
  route: string
  clientVersion: string
  requests: number
  lastSeenAt: number
}

export interface ClientAnalyticsOverview {
  versions: ClientAnalyticsVersionSummary[]
  routes: ClientAnalyticsRouteSummary[]
}

function normalizeTimestamp(date: Date | null): number {
  return date ? date.getTime() : 0
}

export async function readClientAnalyticsOverview(
  { windowMinutes = DEFAULT_WINDOW_MINUTES }: { windowMinutes?: number } = {},
): Promise<ClientAnalyticsOverview> {
  const now = Date.now()
  const from = new Date(now - windowMinutes * 60_000)
  const to = new Date(now)

  const rows = await db
    .select()
    .from(clientRouteMetricMinute)
    .where(and(
      gte(clientRouteMetricMinute.bucketStart, from),
      lte(clientRouteMetricMinute.bucketStart, to),
    ))

  // Aggregate version distribution
  const versionMap = new Map<string, {
    clientVersion: string
    clientPlatform: string | null
    clientArch: string | null
    requests: number
    userIds: Set<string>
    lastSeenAt: number
  }>()

  // Aggregate route usage by version
  const routeMap = new Map<string, {
    method: string
    route: string
    clientVersion: string
    requests: number
    lastSeenAt: number
  }>()

  for (const row of rows) {
    // Version aggregation
    const versionKey = `${row.clientVersion}|${row.clientPlatform ?? ''}|${row.clientArch ?? ''}`
    const existing = versionMap.get(versionKey)
    const rowUserIds = (row.uniqueUserIds ?? []).filter((id): id is string => id !== null)
    const rowLastSeen = normalizeTimestamp(row.lastSeenAt)

    if (existing) {
      existing.requests += row.requests
      for (const uid of rowUserIds) {
        existing.userIds.add(uid)
      }
      existing.lastSeenAt = Math.max(existing.lastSeenAt, rowLastSeen)
    }
    else {
      versionMap.set(versionKey, {
        clientVersion: row.clientVersion,
        clientPlatform: row.clientPlatform,
        clientArch: row.clientArch,
        requests: row.requests,
        userIds: new Set(rowUserIds),
        lastSeenAt: rowLastSeen,
      })
    }

    // Route aggregation
    const routeKey = `${row.method}|${row.route}|${row.clientVersion}`
    const existingRoute = routeMap.get(routeKey)
    if (existingRoute) {
      existingRoute.requests += row.requests
      existingRoute.lastSeenAt = Math.max(existingRoute.lastSeenAt, rowLastSeen)
    }
    else {
      routeMap.set(routeKey, {
        method: row.method,
        route: row.route,
        clientVersion: row.clientVersion,
        requests: row.requests,
        lastSeenAt: rowLastSeen,
      })
    }
  }

  const versions = [...versionMap.values()]
    .map(v => ({
      clientVersion: v.clientVersion,
      clientPlatform: v.clientPlatform,
      clientArch: v.clientArch,
      requests: v.requests,
      uniqueUsers: v.userIds.size,
      lastSeenAt: v.lastSeenAt,
    }))
    .sort((a, b) => b.lastSeenAt - a.lastSeenAt)

  const routes = [...routeMap.values()]
    .sort((a, b) => b.requests - a.requests)

  return { versions, routes }
}

export interface ClientAnalyticsPruneResult {
  retentionDays: number
  cutoff: Date
  deletedRows: number
}

export async function pruneClientAnalytics(
  options: { now?: number, retentionDays?: number } = {},
): Promise<ClientAnalyticsPruneResult> {
  const retentionDays = options.retentionDays ?? env.CLIENT_ANALYTICS_RETENTION_DAYS
  const cutoff = new Date((options.now ?? Date.now()) - retentionDays * 24 * 60 * 60_000)

  const result = await db.execute<{ count: number }>(sql`
    with deleted as (
      delete from client_route_metric_minute
      where bucket_start < ${cutoff}
      returning 1
    )
    select count(*)::int as count from deleted
  `)

  return {
    retentionDays,
    cutoff,
    deletedRows: Number(result.rows[0]?.count ?? 0),
  }
}
