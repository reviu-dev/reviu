import type { SQL } from 'drizzle-orm'
import type { GithubCacheScope } from '../cache/github-cache.js'
import type {
  GithubCacheMetricsOperationDrilldown,
  GithubCacheMetricsOperationDrilldownQuery,
  GithubCacheMetricsOverview,
  GithubCacheMetricsRouteSummary,
  GithubMetricsPersistedSnapshot,
  GithubPersistedOperationMetric,
  GithubPersistedRateLimitState,
  GithubPersistedResourceMetric,
  GithubPersistedUserMetric,
} from './github-metrics.js'
import { and, desc, eq, gte, lte, sql } from 'drizzle-orm'
import { db } from '../../../db/index.js'
import {
  githubOperationMetricMinute,
  githubRateLimitState,
  githubResourceMetricMinute,
  githubUserMetricMinute,
} from '../../../db/schemas/index.js'
import { env } from '../../../lib/env.js'
import { logger } from '../../../lib/logger.js'
import { githubMetricsCollector } from './github-metrics.js'

const DEFAULT_BUCKET_MS = 60_000
const DEFAULT_OVERVIEW_WINDOW_MS = 60 * 60_000
const DEFAULT_OVERVIEW_LIMIT = 10
const GITHUB_RATE_LIMIT_NEAR_THRESHOLD = 0.1

interface GithubMetricsCounters {
  requests: number
  hit: number
  stale: number
  miss: number
  upstreamCalls: number
  notModified: number
  errors: number
  nearLimitEvents: number
  totalBackendDurationMs: number
  totalGithubDurationMs: number
}

interface GithubMetricsOperationAggregate extends GithubMetricsCounters {
  operation: string
  scope?: GithubCacheScope
  ttlMs?: number
  staleMs?: number
  paginatedLoads: number
  totalPageCount: number
  totalItemCount: number
  truncatedCount: number
  totalPaginationDurationMs: number
  lastSeenAt: number
}

interface GithubMetricsUserAggregate {
  userId: string
  requests: number
  upstreamCalls: number
  nearLimitEvents: number
  lowestRemainingPct: number | null
  lastOperation: string | null
  lastSeenAt: number
}

interface PersistedOverviewInput {
  now?: number
  windowMs?: number
  limit?: number
  bucketMs?: number
  operationMetrics: GithubPersistedOperationMetric[]
  resourceMetrics: GithubPersistedResourceMetric[]
  userMetrics: GithubPersistedUserMetric[]
  rateLimitStates: GithubPersistedRateLimitState[]
}

let flushInterval: NodeJS.Timeout | null = null
let flushPromise: Promise<void> | null = null

export interface GithubMetricsPruneCutoffs {
  metricsCutoff: Date
  rateLimitStateCutoff: Date
  metricsRetentionDays: number
  rateLimitStateRetentionDays: number
}

export interface GithubMetricsPruneResult extends GithubMetricsPruneCutoffs {
  deletedOperationMetrics: number
  deletedResourceMetrics: number
  deletedUserMetrics: number
  deletedRateLimitStates: number
  totalDeleted: number
}

function createEmptyCounters(): GithubMetricsCounters {
  return {
    requests: 0,
    hit: 0,
    stale: 0,
    miss: 0,
    upstreamCalls: 0,
    notModified: 0,
    errors: 0,
    nearLimitEvents: 0,
    totalBackendDurationMs: 0,
    totalGithubDurationMs: 0,
  }
}

function createEmptyOperationAggregate(
  operation: string,
  scope: GithubCacheScope | undefined,
  lastSeenAt: number,
): GithubMetricsOperationAggregate {
  return {
    operation,
    scope,
    ...createEmptyCounters(),
    paginatedLoads: 0,
    totalPageCount: 0,
    totalItemCount: 0,
    truncatedCount: 0,
    totalPaginationDurationMs: 0,
    lastSeenAt,
  }
}

function mergeOperationAggregate(
  target: GithubMetricsOperationAggregate,
  source: GithubMetricsOperationAggregate,
) {
  mergeCounters(target, source)
  target.paginatedLoads += source.paginatedLoads
  target.totalPageCount += source.totalPageCount
  target.totalItemCount += source.totalItemCount
  target.truncatedCount += source.truncatedCount
  target.totalPaginationDurationMs += source.totalPaginationDurationMs
  target.ttlMs = source.ttlMs ?? target.ttlMs
  target.staleMs = source.staleMs ?? target.staleMs
  target.lastSeenAt = Math.max(target.lastSeenAt, source.lastSeenAt)
}

function matchesOperationFilter(
  aggregate: Pick<GithubMetricsOperationAggregate, 'operation' | 'scope'>,
  query: Pick<GithubCacheMetricsOperationDrilldownQuery, 'operation' | 'scope'>,
) {
  if (aggregate.operation !== query.operation) {
    return false
  }

  if (query.scope == null) {
    return true
  }

  return aggregate.scope === query.scope
}

function buildRouteSummary(operation: GithubMetricsOperationAggregate): GithubCacheMetricsRouteSummary {
  return {
    operation: operation.operation,
    scope: operation.scope ?? null,
    requests: operation.requests,
    hits: operation.hit,
    staleHits: operation.stale,
    misses: operation.miss,
    hitRate: calculateRate(operation.hit, operation.requests),
    staleRate: calculateRate(operation.stale, operation.requests),
    missRate: calculateRate(operation.miss, operation.requests),
    upstreamCalls: operation.upstreamCalls,
    githubCallsSaved: Math.max(operation.requests - operation.upstreamCalls, 0),
    notModified: operation.notModified,
    notModifiedRate: calculateRate(operation.notModified, operation.upstreamCalls),
    errorCount: operation.errors,
    nearLimitEvents: operation.nearLimitEvents,
    avgBackendDurationMs: operation.requests > 0
      ? operation.totalBackendDurationMs / operation.requests
      : null,
    avgGithubDurationMs: operation.upstreamCalls > 0
      ? operation.totalGithubDurationMs / operation.upstreamCalls
      : null,
    paginatedLoads: operation.paginatedLoads,
    avgPageCount: operation.paginatedLoads > 0
      ? operation.totalPageCount / operation.paginatedLoads
      : null,
    avgItemCount: operation.paginatedLoads > 0
      ? operation.totalItemCount / operation.paginatedLoads
      : null,
    truncatedCount: operation.truncatedCount,
    avgPaginationDurationMs: operation.paginatedLoads > 0
      ? operation.totalPaginationDurationMs / operation.paginatedLoads
      : null,
    ttlMs: operation.ttlMs ?? null,
    staleMs: operation.staleMs ?? null,
    lastSeenAt: operation.lastSeenAt,
  }
}

function buildOperationSeriesPoint(
  bucketStart: number,
  operation: GithubMetricsOperationAggregate,
) {
  return {
    bucketStart,
    requests: operation.requests,
    hit: operation.hit,
    stale: operation.stale,
    miss: operation.miss,
    upstreamCalls: operation.upstreamCalls,
    notModified: operation.notModified,
    errors: operation.errors,
    paginatedLoads: operation.paginatedLoads,
    avgPageCount: operation.paginatedLoads > 0
      ? operation.totalPageCount / operation.paginatedLoads
      : null,
    avgItemCount: operation.paginatedLoads > 0
      ? operation.totalItemCount / operation.paginatedLoads
      : null,
    truncatedCount: operation.truncatedCount,
    avgBackendDurationMs: operation.requests > 0
      ? operation.totalBackendDurationMs / operation.requests
      : null,
    avgGithubDurationMs: operation.upstreamCalls > 0
      ? operation.totalGithubDurationMs / operation.upstreamCalls
      : null,
    avgPaginationDurationMs: operation.paginatedLoads > 0
      ? operation.totalPaginationDurationMs / operation.paginatedLoads
      : null,
  }
}

function calculateRate(numerator: number, denominator: number) {
  if (denominator <= 0) {
    return 0
  }

  return numerator / denominator
}

function buildOperationAggregateKey(operation: string, scope?: GithubCacheScope) {
  return `${operation}:${scope ?? 'unscoped'}`
}

function sortScopes(left: GithubCacheScope, right: GithubCacheScope) {
  const order: GithubCacheScope[] = ['public', 'viewer', 'installation']
  return order.indexOf(left) - order.indexOf(right)
}

function normalizeTimestamp(value: Date | number) {
  return value instanceof Date ? value.getTime() : value
}

function mergeCounters(target: GithubMetricsCounters, source: GithubMetricsCounters) {
  target.requests += source.requests
  target.hit += source.hit
  target.stale += source.stale
  target.miss += source.miss
  target.upstreamCalls += source.upstreamCalls
  target.notModified += source.notModified
  target.errors += source.errors
  target.nearLimitEvents += source.nearLimitEvents
  target.totalBackendDurationMs += source.totalBackendDurationMs
  target.totalGithubDurationMs += source.totalGithubDurationMs
}

function excludedColumn<T extends { name: string }>(column: T) {
  return sql.raw(`excluded.${column.name}`)
}

function daysToMilliseconds(days: number) {
  return days * 24 * 60 * 60_000
}

async function executeDeleteWithCount(query: SQL) {
  const result = await db.execute<{ count: number }>(query)
  return Number(result.rows[0]?.count ?? 0)
}

export function buildGithubMetricsPruneCutoffs(
  {
    now = Date.now(),
    metricsRetentionDays = env.GITHUB_METRICS_RETENTION_DAYS,
    rateLimitStateRetentionDays = env.GITHUB_RATE_LIMIT_STATE_RETENTION_DAYS,
  }: {
    now?: number
    metricsRetentionDays?: number
    rateLimitStateRetentionDays?: number
  } = {},
): GithubMetricsPruneCutoffs {
  return {
    metricsCutoff: new Date(now - daysToMilliseconds(metricsRetentionDays)),
    rateLimitStateCutoff: new Date(now - daysToMilliseconds(rateLimitStateRetentionDays)),
    metricsRetentionDays,
    rateLimitStateRetentionDays,
  }
}

export async function pruneGithubMetrics(
  options: {
    now?: number
    metricsRetentionDays?: number
    rateLimitStateRetentionDays?: number
  } = {},
): Promise<GithubMetricsPruneResult> {
  const cutoffs = buildGithubMetricsPruneCutoffs(options)

  const deletedOperationMetrics = await executeDeleteWithCount(sql<{ count: number }>`
    with deleted as (
      delete from github_operation_metric_minute
      where bucket_start < ${cutoffs.metricsCutoff}
      returning 1
    )
    select count(*)::int as count from deleted
  `)

  const deletedResourceMetrics = await executeDeleteWithCount(sql<{ count: number }>`
    with deleted as (
      delete from github_resource_metric_minute
      where bucket_start < ${cutoffs.metricsCutoff}
      returning 1
    )
    select count(*)::int as count from deleted
  `)

  const deletedUserMetrics = await executeDeleteWithCount(sql<{ count: number }>`
    with deleted as (
      delete from github_user_metric_minute
      where bucket_start < ${cutoffs.metricsCutoff}
      returning 1
    )
    select count(*)::int as count from deleted
  `)

  const deletedRateLimitStates = await executeDeleteWithCount(sql<{ count: number }>`
    with deleted as (
      delete from github_rate_limit_state
      where updated_at < ${cutoffs.rateLimitStateCutoff}
      returning 1
    )
    select count(*)::int as count from deleted
  `)

  return {
    ...cutoffs,
    deletedOperationMetrics,
    deletedResourceMetrics,
    deletedUserMetrics,
    deletedRateLimitStates,
    totalDeleted: deletedOperationMetrics + deletedResourceMetrics + deletedUserMetrics + deletedRateLimitStates,
  }
}

export function buildGithubMetricsOverviewFromPersistedRows(
  {
    now = Date.now(),
    windowMs = DEFAULT_OVERVIEW_WINDOW_MS,
    limit = DEFAULT_OVERVIEW_LIMIT,
    bucketMs = DEFAULT_BUCKET_MS,
    operationMetrics,
    resourceMetrics,
    userMetrics,
    rateLimitStates,
  }: PersistedOverviewInput,
): GithubCacheMetricsOverview {
  const from = now - Math.max(windowMs, bucketMs)
  const normalizedLimit = Math.max(limit, 1)
  const bucketSummaries = new Map<number, GithubMetricsCounters>()
  const operations = new Map<string, GithubMetricsOperationAggregate>()
  const scopeSummary = new Map<GithubCacheScope, GithubMetricsOperationAggregate>()
  const users = new Map<string, GithubMetricsUserAggregate>()
  const resourceSeries: GithubCacheMetricsOverview['githubResourceSeries'] = []

  for (const row of operationMetrics) {
    const bucketStart = normalizeTimestamp(row.bucketStart)
    if (bucketStart < from || bucketStart > now) {
      continue
    }

    const bucketCounters = bucketSummaries.get(bucketStart) ?? createEmptyCounters()
    const aggregate: GithubMetricsCounters = {
      requests: row.requests,
      hit: row.hits,
      stale: row.staleHits,
      miss: row.misses,
      upstreamCalls: row.upstreamCalls,
      notModified: row.notModified,
      errors: row.errorCount,
      nearLimitEvents: row.nearLimitEvents,
      totalBackendDurationMs: row.totalBackendDurationMs,
      totalGithubDurationMs: row.totalGithubDurationMs,
    }
    mergeCounters(bucketCounters, aggregate)
    bucketSummaries.set(bucketStart, bucketCounters)

    const operationKey = buildOperationAggregateKey(row.operation, row.scope)
    const currentOperation = operations.get(operationKey)
    if (!currentOperation) {
      operations.set(operationKey, {
        operation: row.operation,
        scope: row.scope,
        ...aggregate,
        paginatedLoads: row.paginatedLoads,
        totalPageCount: row.totalPageCount,
        totalItemCount: row.totalItemCount,
        truncatedCount: row.truncatedCount,
        totalPaginationDurationMs: row.totalPaginationDurationMs,
        ttlMs: row.ttlMs ?? undefined,
        staleMs: row.staleMs ?? undefined,
        lastSeenAt: normalizeTimestamp(row.lastSeenAt),
      })
    }
    else {
      mergeOperationAggregate(currentOperation, {
        operation: row.operation,
        scope: row.scope,
        ...aggregate,
        paginatedLoads: row.paginatedLoads,
        totalPageCount: row.totalPageCount,
        totalItemCount: row.totalItemCount,
        truncatedCount: row.truncatedCount,
        totalPaginationDurationMs: row.totalPaginationDurationMs,
        ttlMs: row.ttlMs ?? undefined,
        staleMs: row.staleMs ?? undefined,
        lastSeenAt: normalizeTimestamp(row.lastSeenAt),
      })
    }

    const currentScopeSummary = scopeSummary.get(row.scope) ?? createEmptyOperationAggregate(
      row.operation,
      row.scope,
      normalizeTimestamp(row.lastSeenAt),
    )
    mergeOperationAggregate(currentScopeSummary, {
      operation: row.operation,
      scope: row.scope,
      ...aggregate,
      paginatedLoads: row.paginatedLoads,
      totalPageCount: row.totalPageCount,
      totalItemCount: row.totalItemCount,
      truncatedCount: row.truncatedCount,
      totalPaginationDurationMs: row.totalPaginationDurationMs,
      ttlMs: row.ttlMs ?? undefined,
      staleMs: row.staleMs ?? undefined,
      lastSeenAt: normalizeTimestamp(row.lastSeenAt),
    })
    scopeSummary.set(row.scope, currentScopeSummary)
  }

  for (const row of resourceMetrics) {
    const bucketStart = normalizeTimestamp(row.bucketStart)
    if (bucketStart < from || bucketStart > now) {
      continue
    }

    resourceSeries.push({
      bucketStart,
      resource: row.resource,
      upstreamCalls: row.upstreamCalls,
      notModified: row.notModified,
      errors: row.errorCount,
      nearLimitEvents: row.nearLimitEvents,
    })
  }

  for (const row of userMetrics) {
    const bucketStart = normalizeTimestamp(row.bucketStart)
    if (bucketStart < from || bucketStart > now) {
      continue
    }

    const currentUser = users.get(row.userId)
    if (!currentUser) {
      users.set(row.userId, {
        userId: row.userId,
        requests: row.requests,
        upstreamCalls: row.upstreamCalls,
        nearLimitEvents: row.nearLimitEvents,
        lowestRemainingPct: row.lowestRemainingPct,
        lastOperation: row.lastOperation,
        lastSeenAt: normalizeTimestamp(row.lastSeenAt),
      })
      continue
    }

    const shouldReplaceLastOperation = normalizeTimestamp(row.lastSeenAt) >= currentUser.lastSeenAt
    currentUser.requests += row.requests
    currentUser.upstreamCalls += row.upstreamCalls
    currentUser.nearLimitEvents += row.nearLimitEvents
    currentUser.lastSeenAt = Math.max(currentUser.lastSeenAt, normalizeTimestamp(row.lastSeenAt))
    currentUser.lastOperation = shouldReplaceLastOperation ? row.lastOperation : currentUser.lastOperation

    if (row.lowestRemainingPct != null && (currentUser.lowestRemainingPct == null || row.lowestRemainingPct < currentUser.lowestRemainingPct)) {
      currentUser.lowestRemainingPct = row.lowestRemainingPct
    }
  }

  const totals = createEmptyCounters()
  const cacheStatusSeries = [...bucketSummaries.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([bucketStart, counters]) => {
      mergeCounters(totals, counters)
      return {
        bucketStart,
        hit: counters.hit,
        stale: counters.stale,
        miss: counters.miss,
        upstreamCalls: counters.upstreamCalls,
        notModified: counters.notModified,
        errors: counters.errors,
      }
    })

  const normalizedRateLimitStates = rateLimitStates
    .filter(state => normalizeTimestamp(state.updatedAt) >= from)
    .map(state => ({
      ...state,
      updatedAt: normalizeTimestamp(state.updatedAt),
    }))
    .sort((a, b) => {
      const left = a.remainingPct ?? Number.POSITIVE_INFINITY
      const right = b.remainingPct ?? Number.POSITIVE_INFINITY
      return left - right || b.updatedAt - a.updatedAt
    })

  const usersNearLimit = new Set(
    normalizedRateLimitStates
      .filter(state => state.remainingPct != null && state.remainingPct < GITHUB_RATE_LIMIT_NEAR_THRESHOLD)
      .map(state => state.userId),
  ).size

  const paginationTotals = [...operations.values()].reduce((totals, operation) => {
    totals.paginatedLoads += operation.paginatedLoads
    totals.totalPageCount += operation.totalPageCount
    totals.totalItemCount += operation.totalItemCount
    totals.truncatedCount += operation.truncatedCount
    totals.totalPaginationDurationMs += operation.totalPaginationDurationMs
    return totals
  }, {
    paginatedLoads: 0,
    totalPageCount: 0,
    totalItemCount: 0,
    truncatedCount: 0,
    totalPaginationDurationMs: 0,
  })

  return {
    from,
    to: now,
    bucketMs,
    summary: {
      requests: totals.requests,
      hits: totals.hit,
      staleHits: totals.stale,
      misses: totals.miss,
      hitRate: calculateRate(totals.hit, totals.requests),
      staleRate: calculateRate(totals.stale, totals.requests),
      missRate: calculateRate(totals.miss, totals.requests),
      upstreamCalls: totals.upstreamCalls,
      githubCallsSaved: Math.max(totals.requests - totals.upstreamCalls, 0),
      notModified: totals.notModified,
      errorCount: totals.errors,
      nearLimitEvents: totals.nearLimitEvents,
      usersNearLimit,
      paginatedLoads: paginationTotals.paginatedLoads,
      avgPageCount: paginationTotals.paginatedLoads > 0
        ? paginationTotals.totalPageCount / paginationTotals.paginatedLoads
        : null,
      avgItemCount: paginationTotals.paginatedLoads > 0
        ? paginationTotals.totalItemCount / paginationTotals.paginatedLoads
        : null,
      truncatedCount: paginationTotals.truncatedCount,
      avgPaginationDurationMs: paginationTotals.paginatedLoads > 0
        ? paginationTotals.totalPaginationDurationMs / paginationTotals.paginatedLoads
        : null,
    },
    scopeSummary: [...scopeSummary.entries()]
      .sort((a, b) => sortScopes(a[0], b[0]))
      .map(([scope, counters]) => ({
        scope,
        requests: counters.requests,
        hits: counters.hit,
        staleHits: counters.stale,
        misses: counters.miss,
        hitRate: calculateRate(counters.hit, counters.requests),
        staleRate: calculateRate(counters.stale, counters.requests),
        missRate: calculateRate(counters.miss, counters.requests),
        upstreamCalls: counters.upstreamCalls,
        githubCallsSaved: Math.max(counters.requests - counters.upstreamCalls, 0),
        notModified: counters.notModified,
        notModifiedRate: calculateRate(counters.notModified, counters.upstreamCalls),
        errorCount: counters.errors,
        nearLimitEvents: counters.nearLimitEvents,
        avgBackendDurationMs: counters.requests > 0
          ? counters.totalBackendDurationMs / counters.requests
          : null,
        avgGithubDurationMs: counters.upstreamCalls > 0
          ? counters.totalGithubDurationMs / counters.upstreamCalls
          : null,
        paginatedLoads: counters.paginatedLoads,
        avgPageCount: counters.paginatedLoads > 0
          ? counters.totalPageCount / counters.paginatedLoads
          : null,
        avgItemCount: counters.paginatedLoads > 0
          ? counters.totalItemCount / counters.paginatedLoads
          : null,
        truncatedCount: counters.truncatedCount,
        avgPaginationDurationMs: counters.paginatedLoads > 0
          ? counters.totalPaginationDurationMs / counters.paginatedLoads
          : null,
      })),
    cacheStatusSeries,
    githubResourceSeries: resourceSeries.sort((a, b) => a.bucketStart - b.bucketStart || a.resource.localeCompare(b.resource)),
    routes: [...operations.values()]
      .sort((a, b) => b.upstreamCalls - a.upstreamCalls || b.requests - a.requests || a.operation.localeCompare(b.operation))
      .slice(0, normalizedLimit)
      .map(buildRouteSummary),
    users: [...users.values()]
      .sort((a, b) => {
        const leftPct = a.lowestRemainingPct ?? Number.POSITIVE_INFINITY
        const rightPct = b.lowestRemainingPct ?? Number.POSITIVE_INFINITY

        return leftPct - rightPct || b.upstreamCalls - a.upstreamCalls || b.requests - a.requests
      })
      .slice(0, normalizedLimit)
      .map(user => ({
        userId: user.userId,
        requests: user.requests,
        upstreamCalls: user.upstreamCalls,
        nearLimitEvents: user.nearLimitEvents,
        lowestRemainingPct: user.lowestRemainingPct,
        lastOperation: user.lastOperation,
        lastSeenAt: user.lastSeenAt,
      })),
    currentRateLimits: normalizedRateLimitStates.slice(0, normalizedLimit),
  }
}

export function buildGithubMetricsOperationDrilldownFromPersistedRows(
  {
    now = Date.now(),
    windowMs = DEFAULT_OVERVIEW_WINDOW_MS,
    bucketMs = DEFAULT_BUCKET_MS,
    operationMetrics,
    operation,
    scope,
  }: {
    now?: number
    windowMs?: number
    bucketMs?: number
    operationMetrics: GithubPersistedOperationMetric[]
    operation: string
    scope?: GithubCacheScope
  },
): GithubCacheMetricsOperationDrilldown {
  const from = now - Math.max(windowMs, bucketMs)
  const bucketAggregates = new Map<number, GithubMetricsOperationAggregate>()
  let summaryAggregate: GithubMetricsOperationAggregate | null = null

  for (const row of operationMetrics) {
    const bucketStart = normalizeTimestamp(row.bucketStart)
    if (bucketStart < from || bucketStart > now) {
      continue
    }

    if (!matchesOperationFilter({ operation: row.operation, scope: row.scope }, { operation, scope })) {
      continue
    }

    const aggregate: GithubMetricsOperationAggregate = {
      operation: row.operation,
      scope: row.scope,
      requests: row.requests,
      hit: row.hits,
      stale: row.staleHits,
      miss: row.misses,
      upstreamCalls: row.upstreamCalls,
      notModified: row.notModified,
      errors: row.errorCount,
      nearLimitEvents: row.nearLimitEvents,
      totalBackendDurationMs: row.totalBackendDurationMs,
      totalGithubDurationMs: row.totalGithubDurationMs,
      paginatedLoads: row.paginatedLoads,
      totalPageCount: row.totalPageCount,
      totalItemCount: row.totalItemCount,
      truncatedCount: row.truncatedCount,
      totalPaginationDurationMs: row.totalPaginationDurationMs,
      ttlMs: row.ttlMs ?? undefined,
      staleMs: row.staleMs ?? undefined,
      lastSeenAt: normalizeTimestamp(row.lastSeenAt),
    }

    const currentBucketAggregate = bucketAggregates.get(bucketStart)
    if (!currentBucketAggregate) {
      bucketAggregates.set(bucketStart, aggregate)
    }
    else {
      mergeOperationAggregate(currentBucketAggregate, aggregate)
      currentBucketAggregate.scope = scope ?? currentBucketAggregate.scope
    }

    if (!summaryAggregate) {
      summaryAggregate = { ...aggregate }
    }
    else {
      mergeOperationAggregate(summaryAggregate, aggregate)
    }
  }

  if (summaryAggregate && scope == null) {
    summaryAggregate.scope = undefined
  }

  return {
    from,
    to: now,
    bucketMs,
    selection: {
      operation,
      scope: scope ?? null,
    },
    summary: summaryAggregate ? buildRouteSummary(summaryAggregate) : null,
    series: [...bucketAggregates.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([bucketStart, aggregate]) => buildOperationSeriesPoint(bucketStart, aggregate)),
  }
}

async function persistOperationMetrics(metrics: GithubPersistedOperationMetric[]) {
  if (metrics.length === 0) {
    return
  }

  await db
    .insert(githubOperationMetricMinute)
    .values(metrics.map(metric => ({
      bucketStart: new Date(metric.bucketStart),
      operation: metric.operation,
      scope: metric.scope,
      requests: metric.requests,
      hits: metric.hits,
      staleHits: metric.staleHits,
      misses: metric.misses,
      upstreamCalls: metric.upstreamCalls,
      notModified: metric.notModified,
      errorCount: metric.errorCount,
      nearLimitEvents: metric.nearLimitEvents,
      totalBackendDurationMs: metric.totalBackendDurationMs,
      totalGithubDurationMs: metric.totalGithubDurationMs,
      paginatedLoads: metric.paginatedLoads,
      totalPageCount: metric.totalPageCount,
      totalItemCount: metric.totalItemCount,
      truncatedCount: metric.truncatedCount,
      totalPaginationDurationMs: metric.totalPaginationDurationMs,
      ttlMs: metric.ttlMs,
      staleMs: metric.staleMs,
      lastSeenAt: new Date(metric.lastSeenAt),
    })))
    .onConflictDoUpdate({
      target: [
        githubOperationMetricMinute.bucketStart,
        githubOperationMetricMinute.operation,
        githubOperationMetricMinute.scope,
      ],
      set: {
        requests: sql`${githubOperationMetricMinute.requests} + ${excludedColumn(githubOperationMetricMinute.requests)}`,
        hits: sql`${githubOperationMetricMinute.hits} + ${excludedColumn(githubOperationMetricMinute.hits)}`,
        staleHits: sql`${githubOperationMetricMinute.staleHits} + ${excludedColumn(githubOperationMetricMinute.staleHits)}`,
        misses: sql`${githubOperationMetricMinute.misses} + ${excludedColumn(githubOperationMetricMinute.misses)}`,
        upstreamCalls: sql`${githubOperationMetricMinute.upstreamCalls} + ${excludedColumn(githubOperationMetricMinute.upstreamCalls)}`,
        notModified: sql`${githubOperationMetricMinute.notModified} + ${excludedColumn(githubOperationMetricMinute.notModified)}`,
        errorCount: sql`${githubOperationMetricMinute.errorCount} + ${excludedColumn(githubOperationMetricMinute.errorCount)}`,
        nearLimitEvents: sql`${githubOperationMetricMinute.nearLimitEvents} + ${excludedColumn(githubOperationMetricMinute.nearLimitEvents)}`,
        totalBackendDurationMs: sql`${githubOperationMetricMinute.totalBackendDurationMs} + ${excludedColumn(githubOperationMetricMinute.totalBackendDurationMs)}`,
        totalGithubDurationMs: sql`${githubOperationMetricMinute.totalGithubDurationMs} + ${excludedColumn(githubOperationMetricMinute.totalGithubDurationMs)}`,
        paginatedLoads: sql`${githubOperationMetricMinute.paginatedLoads} + ${excludedColumn(githubOperationMetricMinute.paginatedLoads)}`,
        totalPageCount: sql`${githubOperationMetricMinute.totalPageCount} + ${excludedColumn(githubOperationMetricMinute.totalPageCount)}`,
        totalItemCount: sql`${githubOperationMetricMinute.totalItemCount} + ${excludedColumn(githubOperationMetricMinute.totalItemCount)}`,
        truncatedCount: sql`${githubOperationMetricMinute.truncatedCount} + ${excludedColumn(githubOperationMetricMinute.truncatedCount)}`,
        totalPaginationDurationMs: sql`${githubOperationMetricMinute.totalPaginationDurationMs} + ${excludedColumn(githubOperationMetricMinute.totalPaginationDurationMs)}`,
        ttlMs: excludedColumn(githubOperationMetricMinute.ttlMs),
        staleMs: excludedColumn(githubOperationMetricMinute.staleMs),
        lastSeenAt: sql`greatest(${githubOperationMetricMinute.lastSeenAt}, ${excludedColumn(githubOperationMetricMinute.lastSeenAt)})`,
      },
    })
}

async function persistResourceMetrics(metrics: GithubPersistedResourceMetric[]) {
  if (metrics.length === 0) {
    return
  }

  await db
    .insert(githubResourceMetricMinute)
    .values(metrics.map(metric => ({
      bucketStart: new Date(metric.bucketStart),
      resource: metric.resource,
      upstreamCalls: metric.upstreamCalls,
      notModified: metric.notModified,
      errorCount: metric.errorCount,
      nearLimitEvents: metric.nearLimitEvents,
    })))
    .onConflictDoUpdate({
      target: [githubResourceMetricMinute.bucketStart, githubResourceMetricMinute.resource],
      set: {
        upstreamCalls: sql`${githubResourceMetricMinute.upstreamCalls} + ${excludedColumn(githubResourceMetricMinute.upstreamCalls)}`,
        notModified: sql`${githubResourceMetricMinute.notModified} + ${excludedColumn(githubResourceMetricMinute.notModified)}`,
        errorCount: sql`${githubResourceMetricMinute.errorCount} + ${excludedColumn(githubResourceMetricMinute.errorCount)}`,
        nearLimitEvents: sql`${githubResourceMetricMinute.nearLimitEvents} + ${excludedColumn(githubResourceMetricMinute.nearLimitEvents)}`,
      },
    })
}

async function persistUserMetrics(metrics: GithubPersistedUserMetric[]) {
  if (metrics.length === 0) {
    return
  }

  await db
    .insert(githubUserMetricMinute)
    .values(metrics.map(metric => ({
      bucketStart: new Date(metric.bucketStart),
      userId: metric.userId,
      requests: metric.requests,
      upstreamCalls: metric.upstreamCalls,
      nearLimitEvents: metric.nearLimitEvents,
      lowestRemainingPct: metric.lowestRemainingPct,
      lastOperation: metric.lastOperation,
      lastSeenAt: new Date(metric.lastSeenAt),
    })))
    .onConflictDoUpdate({
      target: [githubUserMetricMinute.bucketStart, githubUserMetricMinute.userId],
      set: {
        requests: sql`${githubUserMetricMinute.requests} + ${excludedColumn(githubUserMetricMinute.requests)}`,
        upstreamCalls: sql`${githubUserMetricMinute.upstreamCalls} + ${excludedColumn(githubUserMetricMinute.upstreamCalls)}`,
        nearLimitEvents: sql`${githubUserMetricMinute.nearLimitEvents} + ${excludedColumn(githubUserMetricMinute.nearLimitEvents)}`,
        lowestRemainingPct: sql`
          case
            when ${githubUserMetricMinute.lowestRemainingPct} is null then ${excludedColumn(githubUserMetricMinute.lowestRemainingPct)}
            when ${excludedColumn(githubUserMetricMinute.lowestRemainingPct)} is null then ${githubUserMetricMinute.lowestRemainingPct}
            else least(${githubUserMetricMinute.lowestRemainingPct}, ${excludedColumn(githubUserMetricMinute.lowestRemainingPct)})
          end
        `,
        lastOperation: sql`
          case
            when ${excludedColumn(githubUserMetricMinute.lastSeenAt)} >= ${githubUserMetricMinute.lastSeenAt} then ${excludedColumn(githubUserMetricMinute.lastOperation)}
            else ${githubUserMetricMinute.lastOperation}
          end
        `,
        lastSeenAt: sql`greatest(${githubUserMetricMinute.lastSeenAt}, ${excludedColumn(githubUserMetricMinute.lastSeenAt)})`,
      },
    })
}

async function persistRateLimitStates(rateLimitStates: GithubPersistedRateLimitState[]) {
  if (rateLimitStates.length === 0) {
    return
  }

  await db
    .insert(githubRateLimitState)
    .values(rateLimitStates.map(state => ({
      userId: state.userId,
      resource: state.resource,
      remaining: state.remaining,
      limit: state.limit,
      used: state.used,
      reset: state.reset,
      remainingPct: state.remainingPct,
      lastOperation: state.lastOperation,
      lastRoute: state.lastRoute,
      lastStatus: state.lastStatus,
      updatedAt: new Date(state.updatedAt),
    })))
    .onConflictDoUpdate({
      target: [githubRateLimitState.userId, githubRateLimitState.resource],
      set: {
        remaining: sql`
          case when ${excludedColumn(githubRateLimitState.updatedAt)} >= ${githubRateLimitState.updatedAt}
          then ${excludedColumn(githubRateLimitState.remaining)}
          else ${githubRateLimitState.remaining}
          end
        `,
        limit: sql`
          case when ${excludedColumn(githubRateLimitState.updatedAt)} >= ${githubRateLimitState.updatedAt}
          then ${excludedColumn(githubRateLimitState.limit)}
          else ${githubRateLimitState.limit}
          end
        `,
        used: sql`
          case when ${excludedColumn(githubRateLimitState.updatedAt)} >= ${githubRateLimitState.updatedAt}
          then ${excludedColumn(githubRateLimitState.used)}
          else ${githubRateLimitState.used}
          end
        `,
        reset: sql`
          case when ${excludedColumn(githubRateLimitState.updatedAt)} >= ${githubRateLimitState.updatedAt}
          then ${excludedColumn(githubRateLimitState.reset)}
          else ${githubRateLimitState.reset}
          end
        `,
        remainingPct: sql`
          case when ${excludedColumn(githubRateLimitState.updatedAt)} >= ${githubRateLimitState.updatedAt}
          then ${excludedColumn(githubRateLimitState.remainingPct)}
          else ${githubRateLimitState.remainingPct}
          end
        `,
        lastOperation: sql`
          case when ${excludedColumn(githubRateLimitState.updatedAt)} >= ${githubRateLimitState.updatedAt}
          then ${excludedColumn(githubRateLimitState.lastOperation)}
          else ${githubRateLimitState.lastOperation}
          end
        `,
        lastRoute: sql`
          case when ${excludedColumn(githubRateLimitState.updatedAt)} >= ${githubRateLimitState.updatedAt}
          then ${excludedColumn(githubRateLimitState.lastRoute)}
          else ${githubRateLimitState.lastRoute}
          end
        `,
        lastStatus: sql`
          case when ${excludedColumn(githubRateLimitState.updatedAt)} >= ${githubRateLimitState.updatedAt}
          then ${excludedColumn(githubRateLimitState.lastStatus)}
          else ${githubRateLimitState.lastStatus}
          end
        `,
        updatedAt: sql`greatest(${githubRateLimitState.updatedAt}, ${excludedColumn(githubRateLimitState.updatedAt)})`,
      },
    })
}

async function persistGithubMetricsSnapshot(snapshot: GithubMetricsPersistedSnapshot) {
  await persistOperationMetrics(snapshot.operationMetrics)
  await persistResourceMetrics(snapshot.resourceMetrics)
  await persistUserMetrics(snapshot.userMetrics)
  await persistRateLimitStates(snapshot.rateLimitStates)
}

export async function flushGithubMetricsNow() {
  if (flushPromise) {
    return flushPromise
  }

  flushPromise = (async () => {
    const snapshot = githubMetricsCollector.drainPersistedMetrics()
    const hasData = snapshot.operationMetrics.length > 0
      || snapshot.resourceMetrics.length > 0
      || snapshot.userMetrics.length > 0
      || snapshot.rateLimitStates.length > 0

    if (!hasData) {
      return
    }

    try {
      await persistGithubMetricsSnapshot(snapshot)
    }
    catch (error) {
      githubMetricsCollector.requeuePersistedMetrics(snapshot)
      throw error
    }
  })()
    .catch((error) => {
      logger.warn({ error }, 'Failed to flush GitHub metrics to Postgres')
      throw error
    })
    .finally(() => {
      flushPromise = null
    })

  return flushPromise
}

export function startGithubMetricsPersistence() {
  if (flushInterval) {
    return
  }

  flushInterval = setInterval(() => {
    void flushGithubMetricsNow().catch(() => undefined)
  }, env.GITHUB_METRICS_FLUSH_INTERVAL_MS)

  flushInterval.unref?.()
}

export function stopGithubMetricsPersistence() {
  if (!flushInterval) {
    return
  }

  clearInterval(flushInterval)
  flushInterval = null
}

export async function readGithubMetricsOverviewFromDatabase(
  {
    now = Date.now(),
    windowMs = DEFAULT_OVERVIEW_WINDOW_MS,
    limit = DEFAULT_OVERVIEW_LIMIT,
  }: {
    now?: number
    windowMs?: number
    limit?: number
  } = {},
) {
  const boundedWindowMs = Math.max(windowMs, DEFAULT_BUCKET_MS)
  const from = now - boundedWindowMs

  const [operationRows, resourceRows, userRows, rateLimitRows] = await Promise.all([
    db.select().from(githubOperationMetricMinute).where(and(
      gte(githubOperationMetricMinute.bucketStart, new Date(from)),
      lte(githubOperationMetricMinute.bucketStart, new Date(now)),
    )),
    db.select().from(githubResourceMetricMinute).where(and(
      gte(githubResourceMetricMinute.bucketStart, new Date(from)),
      lte(githubResourceMetricMinute.bucketStart, new Date(now)),
    )),
    db.select().from(githubUserMetricMinute).where(and(
      gte(githubUserMetricMinute.bucketStart, new Date(from)),
      lte(githubUserMetricMinute.bucketStart, new Date(now)),
    )),
    db.select().from(githubRateLimitState).where(gte(githubRateLimitState.updatedAt, new Date(from))).orderBy(desc(githubRateLimitState.updatedAt)),
  ])

  return buildGithubMetricsOverviewFromPersistedRows({
    now,
    windowMs: boundedWindowMs,
    limit,
    bucketMs: DEFAULT_BUCKET_MS,
    operationMetrics: operationRows.map(row => ({
      bucketStart: normalizeTimestamp(row.bucketStart),
      operation: row.operation,
      scope: row.scope as GithubCacheScope,
      requests: row.requests,
      hits: row.hits,
      staleHits: row.staleHits,
      misses: row.misses,
      upstreamCalls: row.upstreamCalls,
      notModified: row.notModified,
      errorCount: row.errorCount,
      nearLimitEvents: row.nearLimitEvents,
      totalBackendDurationMs: row.totalBackendDurationMs,
      totalGithubDurationMs: row.totalGithubDurationMs,
      paginatedLoads: row.paginatedLoads,
      totalPageCount: row.totalPageCount,
      totalItemCount: row.totalItemCount,
      truncatedCount: row.truncatedCount,
      totalPaginationDurationMs: row.totalPaginationDurationMs,
      ttlMs: row.ttlMs,
      staleMs: row.staleMs,
      lastSeenAt: normalizeTimestamp(row.lastSeenAt),
    })),
    resourceMetrics: resourceRows.map(row => ({
      bucketStart: normalizeTimestamp(row.bucketStart),
      resource: row.resource,
      upstreamCalls: row.upstreamCalls,
      notModified: row.notModified,
      errorCount: row.errorCount,
      nearLimitEvents: row.nearLimitEvents,
    })),
    userMetrics: userRows.map(row => ({
      bucketStart: normalizeTimestamp(row.bucketStart),
      userId: row.userId,
      requests: row.requests,
      upstreamCalls: row.upstreamCalls,
      nearLimitEvents: row.nearLimitEvents,
      lowestRemainingPct: row.lowestRemainingPct,
      lastOperation: row.lastOperation,
      lastSeenAt: normalizeTimestamp(row.lastSeenAt),
    })),
    rateLimitStates: rateLimitRows.map(row => ({
      userId: row.userId,
      resource: row.resource,
      remaining: row.remaining,
      limit: row.limit,
      used: row.used,
      reset: row.reset,
      remainingPct: row.remainingPct,
      lastOperation: row.lastOperation,
      lastRoute: row.lastRoute,
      lastStatus: row.lastStatus,
      updatedAt: normalizeTimestamp(row.updatedAt),
    })),
  })
}

export async function readGithubMetricsOperationDrilldownFromDatabase(
  {
    now = Date.now(),
    windowMs = DEFAULT_OVERVIEW_WINDOW_MS,
    operation,
    scope,
  }: {
    now?: number
    windowMs?: number
    operation: string
    scope?: GithubCacheScope
  },
) {
  const boundedWindowMs = Math.max(windowMs, DEFAULT_BUCKET_MS)
  const from = now - boundedWindowMs
  const whereCondition = scope == null
    ? and(
        eq(githubOperationMetricMinute.operation, operation),
        gte(githubOperationMetricMinute.bucketStart, new Date(from)),
        lte(githubOperationMetricMinute.bucketStart, new Date(now)),
      )
    : and(
        eq(githubOperationMetricMinute.operation, operation),
        eq(githubOperationMetricMinute.scope, scope),
        gte(githubOperationMetricMinute.bucketStart, new Date(from)),
        lte(githubOperationMetricMinute.bucketStart, new Date(now)),
      )

  const operationRows = await db
    .select()
    .from(githubOperationMetricMinute)
    .where(whereCondition)

  return buildGithubMetricsOperationDrilldownFromPersistedRows({
    now,
    windowMs: boundedWindowMs,
    bucketMs: DEFAULT_BUCKET_MS,
    operation,
    scope,
    operationMetrics: operationRows.map(row => ({
      bucketStart: normalizeTimestamp(row.bucketStart),
      operation: row.operation,
      scope: row.scope as GithubCacheScope,
      requests: row.requests,
      hits: row.hits,
      staleHits: row.staleHits,
      misses: row.misses,
      upstreamCalls: row.upstreamCalls,
      notModified: row.notModified,
      errorCount: row.errorCount,
      nearLimitEvents: row.nearLimitEvents,
      totalBackendDurationMs: row.totalBackendDurationMs,
      totalGithubDurationMs: row.totalGithubDurationMs,
      paginatedLoads: row.paginatedLoads,
      totalPageCount: row.totalPageCount,
      totalItemCount: row.totalItemCount,
      truncatedCount: row.truncatedCount,
      totalPaginationDurationMs: row.totalPaginationDurationMs,
      ttlMs: row.ttlMs,
      staleMs: row.staleMs,
      lastSeenAt: normalizeTimestamp(row.lastSeenAt),
    })),
  })
}
