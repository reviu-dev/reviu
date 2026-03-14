import type { GithubCacheScope, GithubCacheStatus } from '../cache/github-cache.js'
import type { GithubRateLimitInfo } from '../service.js'

export interface GithubCacheMetricEvent {
  at?: number
  userId?: string
  operation: string
  scope: GithubCacheScope
  cacheStatus: GithubCacheStatus
  ttlMs: number
  staleMs: number
  durationMs: number
}

export interface GithubApiMetricEvent {
  at?: number
  userId?: string
  operation: string
  scope?: GithubCacheScope
  route: string
  status: number
  durationMs: number
  notModified?: boolean
  rateLimit?: GithubRateLimitInfo | null
}

export interface GithubPaginationMetricEvent {
  at?: number
  userId?: string
  operation: string
  scope?: GithubCacheScope
  pageCount: number
  itemCount: number
  truncated: boolean
  durationMs: number
}

export interface GithubCacheMetricsOverviewQuery {
  now?: number
  windowMs?: number
  limit?: number
}

export interface GithubCacheMetricsOperationDrilldownQuery {
  now?: number
  windowMs?: number
  operation: string
  scope?: GithubCacheScope
}

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

interface GithubMetricsResourceAggregate {
  resource: string
  upstreamCalls: number
  notModified: number
  errors: number
  nearLimitEvents: number
}

interface GithubRateLimitState {
  userId: string
  resource: string
  remaining: number | null
  limit: number | null
  used: number | null
  reset: number | null
  remainingPct: number | null
  lastOperation: string | null
  lastRoute: string | null
  lastStatus: number
  updatedAt: number
}

interface GithubMetricsBucket {
  bucketStart: number
  summary: GithubMetricsCounters
  operations: Map<string, GithubMetricsOperationAggregate>
  users: Map<string, GithubMetricsUserAggregate>
  resources: Map<string, GithubMetricsResourceAggregate>
}

export interface GithubPersistedOperationMetric {
  bucketStart: number
  operation: string
  scope: GithubCacheScope
  requests: number
  hits: number
  staleHits: number
  misses: number
  upstreamCalls: number
  notModified: number
  errorCount: number
  nearLimitEvents: number
  totalBackendDurationMs: number
  totalGithubDurationMs: number
  paginatedLoads: number
  totalPageCount: number
  totalItemCount: number
  truncatedCount: number
  totalPaginationDurationMs: number
  ttlMs: number | null
  staleMs: number | null
  lastSeenAt: number
}

export interface GithubPersistedResourceMetric {
  bucketStart: number
  resource: string
  upstreamCalls: number
  notModified: number
  errorCount: number
  nearLimitEvents: number
}

export interface GithubPersistedUserMetric {
  bucketStart: number
  userId: string
  requests: number
  upstreamCalls: number
  nearLimitEvents: number
  lowestRemainingPct: number | null
  lastOperation: string | null
  lastSeenAt: number
}

export interface GithubPersistedRateLimitState {
  userId: string
  resource: string
  remaining: number | null
  limit: number | null
  used: number | null
  reset: number | null
  remainingPct: number | null
  lastOperation: string | null
  lastRoute: string | null
  lastStatus: number
  updatedAt: number
}

export interface GithubMetricsPersistedSnapshot {
  bucketMs: number
  operationMetrics: GithubPersistedOperationMetric[]
  resourceMetrics: GithubPersistedResourceMetric[]
  userMetrics: GithubPersistedUserMetric[]
  rateLimitStates: GithubPersistedRateLimitState[]
}

function mergePersistedOperationMetric(
  target: GithubPersistedOperationMetric,
  source: GithubPersistedOperationMetric,
) {
  target.requests += source.requests
  target.hits += source.hits
  target.staleHits += source.staleHits
  target.misses += source.misses
  target.upstreamCalls += source.upstreamCalls
  target.notModified += source.notModified
  target.errorCount += source.errorCount
  target.nearLimitEvents += source.nearLimitEvents
  target.totalBackendDurationMs += source.totalBackendDurationMs
  target.totalGithubDurationMs += source.totalGithubDurationMs
  target.paginatedLoads += source.paginatedLoads
  target.totalPageCount += source.totalPageCount
  target.totalItemCount += source.totalItemCount
  target.truncatedCount += source.truncatedCount
  target.totalPaginationDurationMs += source.totalPaginationDurationMs
  target.ttlMs = source.ttlMs ?? target.ttlMs
  target.staleMs = source.staleMs ?? target.staleMs
  target.lastSeenAt = Math.max(target.lastSeenAt, source.lastSeenAt)
}

function mergePersistedResourceMetric(
  target: GithubPersistedResourceMetric,
  source: GithubPersistedResourceMetric,
) {
  target.upstreamCalls += source.upstreamCalls
  target.notModified += source.notModified
  target.errorCount += source.errorCount
  target.nearLimitEvents += source.nearLimitEvents
}

function mergePersistedUserMetric(
  target: GithubPersistedUserMetric,
  source: GithubPersistedUserMetric,
) {
  const shouldReplaceLastOperation = source.lastSeenAt >= target.lastSeenAt

  target.requests += source.requests
  target.upstreamCalls += source.upstreamCalls
  target.nearLimitEvents += source.nearLimitEvents
  target.lastSeenAt = Math.max(target.lastSeenAt, source.lastSeenAt)
  target.lastOperation = shouldReplaceLastOperation
    ? source.lastOperation
    : target.lastOperation

  if (source.lowestRemainingPct != null && (target.lowestRemainingPct == null || source.lowestRemainingPct < target.lowestRemainingPct)) {
    target.lowestRemainingPct = source.lowestRemainingPct
  }
}

export interface GithubCacheMetricsOverview {
  from: number
  to: number
  bucketMs: number
  summary: {
    requests: number
    hits: number
    staleHits: number
    misses: number
    hitRate: number
    staleRate: number
    missRate: number
    upstreamCalls: number
    githubCallsSaved: number
    notModified: number
    errorCount: number
    nearLimitEvents: number
    usersNearLimit: number
    paginatedLoads: number
    avgPageCount: number | null
    avgItemCount: number | null
    truncatedCount: number
    avgPaginationDurationMs: number | null
  }
  scopeSummary: Array<{
    scope: GithubCacheScope
    requests: number
    hits: number
    staleHits: number
    misses: number
    hitRate: number
    staleRate: number
    missRate: number
    upstreamCalls: number
    githubCallsSaved: number
    notModified: number
    notModifiedRate: number
    errorCount: number
    nearLimitEvents: number
    avgBackendDurationMs: number | null
    avgGithubDurationMs: number | null
    paginatedLoads: number
    avgPageCount: number | null
    avgItemCount: number | null
    truncatedCount: number
    avgPaginationDurationMs: number | null
  }>
  scopeSeries: Array<{
    bucketStart: number
    scope: GithubCacheScope
    requests: number
    upstreamCalls: number
    githubCallsSaved: number
    hit: number
    stale: number
    miss: number
  }>
  cacheStatusSeries: Array<{
    bucketStart: number
    hit: number
    stale: number
    miss: number
    upstreamCalls: number
    notModified: number
    errors: number
  }>
  githubResourceSeries: Array<{
    bucketStart: number
    resource: string
    upstreamCalls: number
    notModified: number
    errors: number
    nearLimitEvents: number
  }>
  routes: GithubCacheMetricsRouteSummary[]
  users: Array<{
    userId: string
    requests: number
    upstreamCalls: number
    nearLimitEvents: number
    lowestRemainingPct: number | null
    lastOperation: string | null
    lastSeenAt: number
  }>
  currentRateLimits: Array<{
    userId: string
    resource: string
    remaining: number | null
    limit: number | null
    used: number | null
    reset: number | null
    remainingPct: number | null
    lastOperation: string | null
    lastRoute: string | null
    lastStatus: number
    updatedAt: number
  }>
}

export interface GithubCacheMetricsRouteSummary {
  operation: string
  scope: GithubCacheScope | null
  requests: number
  hits: number
  staleHits: number
  misses: number
  hitRate: number
  staleRate: number
  missRate: number
  upstreamCalls: number
  githubCallsSaved: number
  notModified: number
  notModifiedRate: number
  errorCount: number
  nearLimitEvents: number
  avgBackendDurationMs: number | null
  avgGithubDurationMs: number | null
  paginatedLoads: number
  avgPageCount: number | null
  avgItemCount: number | null
  truncatedCount: number
  avgPaginationDurationMs: number | null
  ttlMs: number | null
  staleMs: number | null
  lastSeenAt: number
}

export interface GithubCacheMetricsOperationSeriesPoint {
  bucketStart: number
  requests: number
  hit: number
  stale: number
  miss: number
  upstreamCalls: number
  notModified: number
  errors: number
  paginatedLoads: number
  avgPageCount: number | null
  avgItemCount: number | null
  truncatedCount: number
  avgBackendDurationMs: number | null
  avgGithubDurationMs: number | null
  avgPaginationDurationMs: number | null
}

export interface GithubCacheMetricsOperationDrilldown {
  from: number
  to: number
  bucketMs: number
  selection: {
    operation: string
    scope: GithubCacheScope | null
  }
  summary: GithubCacheMetricsRouteSummary | null
  series: GithubCacheMetricsOperationSeriesPoint[]
}

const DEFAULT_BUCKET_MS = 60_000
const DEFAULT_RETENTION_MS = 24 * 60 * 60_000
const DEFAULT_OVERVIEW_WINDOW_MS = 60 * 60_000
const DEFAULT_OVERVIEW_LIMIT = 10
const GITHUB_RATE_LIMIT_NEAR_THRESHOLD = 0.1

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
): GithubCacheMetricsOperationSeriesPoint {
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

function calculateRemainingPct(rateLimit: GithubRateLimitInfo | null | undefined) {
  if (!rateLimit || rateLimit.limit == null || rateLimit.remaining == null || rateLimit.limit <= 0) {
    return null
  }

  return rateLimit.remaining / rateLimit.limit
}

function isNearLimit(rateLimit: GithubRateLimitInfo | null | undefined) {
  const remainingPct = calculateRemainingPct(rateLimit)
  return remainingPct != null && remainingPct < GITHUB_RATE_LIMIT_NEAR_THRESHOLD
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

export function createGithubMetricsCollector(
  {
    now = () => Date.now(),
    bucketMs = DEFAULT_BUCKET_MS,
    retentionMs = DEFAULT_RETENTION_MS,
  }: {
    now?: () => number
    bucketMs?: number
    retentionMs?: number
  } = {},
) {
  return new GithubMetricsCollector(now, bucketMs, retentionMs)
}

export class GithubMetricsCollector {
  private readonly buckets = new Map<number, GithubMetricsBucket>()
  private readonly currentRateLimits = new Map<string, GithubRateLimitState>()
  private readonly pendingOperationMetrics = new Map<string, GithubPersistedOperationMetric>()
  private readonly pendingResourceMetrics = new Map<string, GithubPersistedResourceMetric>()
  private readonly pendingUserMetrics = new Map<string, GithubPersistedUserMetric>()
  private readonly pendingRateLimitStates = new Map<string, GithubPersistedRateLimitState>()

  constructor(
    private readonly now: () => number,
    private readonly bucketMs: number,
    private readonly retentionMs: number,
  ) {}

  recordCacheEvent(event: GithubCacheMetricEvent) {
    const at = event.at ?? this.now()
    const bucketStart = this.getBucketStart(at)
    const bucket = this.getOrCreateBucket(at)
    const operation = this.getOrCreateOperation(bucket, event.operation, event.scope, at)
    const pendingOperation = this.getOrCreatePendingOperation(bucketStart, event.operation, event.scope, at)

    bucket.summary.requests += 1
    bucket.summary.totalBackendDurationMs += event.durationMs

    operation.requests += 1
    operation.totalBackendDurationMs += event.durationMs
    operation.scope = event.scope
    operation.ttlMs = event.ttlMs
    operation.staleMs = event.staleMs
    operation.lastSeenAt = at

    pendingOperation.requests += 1
    pendingOperation.totalBackendDurationMs += event.durationMs
    pendingOperation.ttlMs = event.ttlMs
    pendingOperation.staleMs = event.staleMs
    pendingOperation.lastSeenAt = at

    if (event.cacheStatus === 'hit') {
      bucket.summary.hit += 1
      operation.hit += 1
      pendingOperation.hits += 1
    }
    else if (event.cacheStatus === 'stale') {
      bucket.summary.stale += 1
      operation.stale += 1
      pendingOperation.staleHits += 1
    }
    else {
      bucket.summary.miss += 1
      operation.miss += 1
      pendingOperation.misses += 1
    }

    if (event.userId) {
      const user = this.getOrCreateUser(bucket, event.userId, at)
      const pendingUser = this.getOrCreatePendingUser(bucketStart, event.userId, at)
      user.requests += 1
      user.lastOperation = event.operation
      user.lastSeenAt = at

      pendingUser.requests += 1
      pendingUser.lastOperation = event.operation
      pendingUser.lastSeenAt = at
    }
  }

  recordGithubApiEvent(event: GithubApiMetricEvent) {
    const at = event.at ?? this.now()
    const bucketStart = this.getBucketStart(at)
    const bucket = this.getOrCreateBucket(at)
    const operation = this.getOrCreateOperation(bucket, event.operation, event.scope, at)
    const resource = event.rateLimit?.resource ?? 'unknown'
    const resourceAggregate = this.getOrCreateResource(bucket, resource)
    const pendingOperation = event.scope
      ? this.getOrCreatePendingOperation(bucketStart, event.operation, event.scope, at)
      : null
    const pendingResource = this.getOrCreatePendingResource(bucketStart, resource)
    const nearLimit = isNearLimit(event.rateLimit)
    const error = event.status >= 400 && event.status !== 304

    bucket.summary.upstreamCalls += 1
    bucket.summary.totalGithubDurationMs += event.durationMs

    operation.upstreamCalls += 1
    operation.totalGithubDurationMs += event.durationMs
    operation.lastSeenAt = at

    if (pendingOperation) {
      pendingOperation.upstreamCalls += 1
      pendingOperation.totalGithubDurationMs += event.durationMs
      pendingOperation.lastSeenAt = at
    }

    resourceAggregate.upstreamCalls += 1
    pendingResource.upstreamCalls += 1

    if (event.notModified) {
      bucket.summary.notModified += 1
      operation.notModified += 1
      resourceAggregate.notModified += 1
      pendingOperation && pendingOperation.notModified++
      pendingResource.notModified += 1
    }

    if (error) {
      bucket.summary.errors += 1
      operation.errors += 1
      resourceAggregate.errors += 1
      pendingOperation && pendingOperation.errorCount++
      pendingResource.errorCount += 1
    }

    if (nearLimit) {
      bucket.summary.nearLimitEvents += 1
      operation.nearLimitEvents += 1
      resourceAggregate.nearLimitEvents += 1
      pendingOperation && pendingOperation.nearLimitEvents++
      pendingResource.nearLimitEvents += 1
    }

    if (event.userId) {
      const user = this.getOrCreateUser(bucket, event.userId, at)
      const pendingUser = this.getOrCreatePendingUser(bucketStart, event.userId, at)
      user.upstreamCalls += 1
      user.lastOperation = event.operation
      user.lastSeenAt = at

      pendingUser.upstreamCalls += 1
      pendingUser.lastOperation = event.operation
      pendingUser.lastSeenAt = at

      if (nearLimit) {
        user.nearLimitEvents += 1
        pendingUser.nearLimitEvents += 1
      }

      const remainingPct = calculateRemainingPct(event.rateLimit)
      if (remainingPct != null && (user.lowestRemainingPct == null || remainingPct < user.lowestRemainingPct)) {
        user.lowestRemainingPct = remainingPct
      }
      if (remainingPct != null && (pendingUser.lowestRemainingPct == null || remainingPct < pendingUser.lowestRemainingPct)) {
        pendingUser.lowestRemainingPct = remainingPct
      }

      if (event.rateLimit) {
        const rateLimitState = {
          userId: event.userId,
          resource,
          remaining: event.rateLimit.remaining ?? null,
          limit: event.rateLimit.limit ?? null,
          used: event.rateLimit.used ?? null,
          reset: event.rateLimit.reset ?? null,
          remainingPct,
          lastOperation: event.operation,
          lastRoute: event.route,
          lastStatus: event.status,
          updatedAt: at,
        } satisfies GithubPersistedRateLimitState

        this.currentRateLimits.set(`${event.userId}:${resource}`, rateLimitState)
        this.pendingRateLimitStates.set(`${event.userId}:${resource}`, rateLimitState)
      }
    }
  }

  recordPaginationEvent(event: GithubPaginationMetricEvent) {
    const at = event.at ?? this.now()
    if (!event.scope) {
      return
    }

    const bucketStart = this.getBucketStart(at)
    const bucket = this.getOrCreateBucket(at)
    const operation = this.getOrCreateOperation(bucket, event.operation, event.scope, at)
    const pendingOperation = this.getOrCreatePendingOperation(bucketStart, event.operation, event.scope, at)

    operation.paginatedLoads += 1
    operation.totalPageCount += event.pageCount
    operation.totalItemCount += event.itemCount
    operation.totalPaginationDurationMs += event.durationMs
    operation.lastSeenAt = at

    pendingOperation.paginatedLoads += 1
    pendingOperation.totalPageCount += event.pageCount
    pendingOperation.totalItemCount += event.itemCount
    pendingOperation.totalPaginationDurationMs += event.durationMs
    pendingOperation.lastSeenAt = at

    if (event.truncated) {
      operation.truncatedCount += 1
      pendingOperation.truncatedCount += 1
    }
  }

  drainPersistedMetrics(): GithubMetricsPersistedSnapshot {
    const snapshot: GithubMetricsPersistedSnapshot = {
      bucketMs: this.bucketMs,
      operationMetrics: [...this.pendingOperationMetrics.values()].map(metric => ({ ...metric })),
      resourceMetrics: [...this.pendingResourceMetrics.values()].map(metric => ({ ...metric })),
      userMetrics: [...this.pendingUserMetrics.values()].map(metric => ({ ...metric })),
      rateLimitStates: [...this.pendingRateLimitStates.values()].map(metric => ({ ...metric })),
    }

    this.pendingOperationMetrics.clear()
    this.pendingResourceMetrics.clear()
    this.pendingUserMetrics.clear()
    this.pendingRateLimitStates.clear()

    return snapshot
  }

  requeuePersistedMetrics(snapshot: GithubMetricsPersistedSnapshot) {
    for (const metric of snapshot.operationMetrics) {
      const key = `${metric.bucketStart}:${buildOperationAggregateKey(metric.operation, metric.scope)}`
      const current = this.pendingOperationMetrics.get(key)
      if (current) {
        mergePersistedOperationMetric(current, metric)
        continue
      }

      this.pendingOperationMetrics.set(key, { ...metric })
    }

    for (const metric of snapshot.resourceMetrics) {
      const key = `${metric.bucketStart}:${metric.resource}`
      const current = this.pendingResourceMetrics.get(key)
      if (current) {
        mergePersistedResourceMetric(current, metric)
        continue
      }

      this.pendingResourceMetrics.set(key, { ...metric })
    }

    for (const metric of snapshot.userMetrics) {
      const key = `${metric.bucketStart}:${metric.userId}`
      const current = this.pendingUserMetrics.get(key)
      if (current) {
        mergePersistedUserMetric(current, metric)
        continue
      }

      this.pendingUserMetrics.set(key, { ...metric })
    }

    for (const metric of snapshot.rateLimitStates) {
      const key = `${metric.userId}:${metric.resource}`
      const current = this.pendingRateLimitStates.get(key)
      if (!current || metric.updatedAt >= current.updatedAt) {
        this.pendingRateLimitStates.set(key, { ...metric })
      }
    }
  }

  getOverview(query: GithubCacheMetricsOverviewQuery = {}): GithubCacheMetricsOverview {
    const now = query.now ?? this.now()
    const windowMs = Math.max(query.windowMs ?? DEFAULT_OVERVIEW_WINDOW_MS, this.bucketMs)
    const limit = Math.max(query.limit ?? DEFAULT_OVERVIEW_LIMIT, 1)
    const from = now - windowMs

    this.prune(now)

    const buckets = [...this.buckets.values()]
      .filter(bucket => bucket.bucketStart >= from && bucket.bucketStart <= now)
      .sort((a, b) => a.bucketStart - b.bucketStart)

    const totals = createEmptyCounters()
    const operations = new Map<string, GithubMetricsOperationAggregate>()
    const scopeSummary = new Map<GithubCacheScope, GithubMetricsOperationAggregate>()
    const scopeSeries: GithubCacheMetricsOverview['scopeSeries'] = []
    const users = new Map<string, GithubMetricsUserAggregate>()
    const resourceSeries: GithubCacheMetricsOverview['githubResourceSeries'] = []

    for (const bucket of buckets) {
      mergeCounters(totals, bucket.summary)
      const bucketScopeSummary = new Map<GithubCacheScope, GithubMetricsOperationAggregate>()

      for (const resource of bucket.resources.values()) {
        resourceSeries.push({
          bucketStart: bucket.bucketStart,
          resource: resource.resource,
          upstreamCalls: resource.upstreamCalls,
          notModified: resource.notModified,
          errors: resource.errors,
          nearLimitEvents: resource.nearLimitEvents,
        })
      }

      for (const aggregate of bucket.operations.values()) {
        const aggregateKey = buildOperationAggregateKey(aggregate.operation, aggregate.scope)
        const current = operations.get(aggregateKey)
        if (!current) {
          operations.set(aggregateKey, { ...aggregate })
          continue
        }

        mergeOperationAggregate(current, aggregate)
        current.scope = aggregate.scope ?? current.scope
      }

      for (const aggregate of bucket.operations.values()) {
        if (!aggregate.scope) {
          continue
        }

        const current = scopeSummary.get(aggregate.scope) ?? createEmptyOperationAggregate(
          aggregate.operation,
          aggregate.scope,
          aggregate.lastSeenAt,
        )
        mergeOperationAggregate(current, aggregate)
        scopeSummary.set(aggregate.scope, current)

        const currentBucketScope = bucketScopeSummary.get(aggregate.scope) ?? createEmptyOperationAggregate(
          aggregate.operation,
          aggregate.scope,
          aggregate.lastSeenAt,
        )
        mergeOperationAggregate(currentBucketScope, aggregate)
        bucketScopeSummary.set(aggregate.scope, currentBucketScope)
      }

      scopeSeries.push(
        ...[...bucketScopeSummary.entries()]
          .sort((a, b) => sortScopes(a[0], b[0]))
          .map(([scope, counters]) => ({
            bucketStart: bucket.bucketStart,
            scope,
            requests: counters.requests,
            upstreamCalls: counters.upstreamCalls,
            githubCallsSaved: Math.max(counters.requests - counters.upstreamCalls, 0),
            hit: counters.hit,
            stale: counters.stale,
            miss: counters.miss,
          })),
      )

      for (const aggregate of bucket.users.values()) {
        const current = users.get(aggregate.userId)
        if (!current) {
          users.set(aggregate.userId, { ...aggregate })
          continue
        }

        const shouldReplaceLastOperation = aggregate.lastSeenAt >= current.lastSeenAt
        current.requests += aggregate.requests
        current.upstreamCalls += aggregate.upstreamCalls
        current.nearLimitEvents += aggregate.nearLimitEvents
        current.lastSeenAt = Math.max(current.lastSeenAt, aggregate.lastSeenAt)
        current.lastOperation = shouldReplaceLastOperation
          ? aggregate.lastOperation
          : current.lastOperation

        if (
          aggregate.lowestRemainingPct != null
          && (current.lowestRemainingPct == null || aggregate.lowestRemainingPct < current.lowestRemainingPct)
        ) {
          current.lowestRemainingPct = aggregate.lowestRemainingPct
        }
      }
    }

    const currentRateLimits = [...this.currentRateLimits.values()]
      .filter(rateLimit => rateLimit.updatedAt >= from)
      .sort((a, b) => {
        const left = a.remainingPct ?? Number.POSITIVE_INFINITY
        const right = b.remainingPct ?? Number.POSITIVE_INFINITY
        return left - right || b.updatedAt - a.updatedAt
      })

    const usersNearLimit = new Set(
      currentRateLimits
        .filter(rateLimit => rateLimit.remainingPct != null && rateLimit.remainingPct < GITHUB_RATE_LIMIT_NEAR_THRESHOLD)
        .map(rateLimit => rateLimit.userId),
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
      bucketMs: this.bucketMs,
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
      scopeSeries: scopeSeries.sort((a, b) => a.bucketStart - b.bucketStart || sortScopes(a.scope, b.scope)),
      cacheStatusSeries: buckets.map(bucket => ({
        bucketStart: bucket.bucketStart,
        hit: bucket.summary.hit,
        stale: bucket.summary.stale,
        miss: bucket.summary.miss,
        upstreamCalls: bucket.summary.upstreamCalls,
        notModified: bucket.summary.notModified,
        errors: bucket.summary.errors,
      })),
      githubResourceSeries: resourceSeries.sort((a, b) => a.bucketStart - b.bucketStart || a.resource.localeCompare(b.resource)),
      routes: [...operations.values()]
        .sort((a, b) => {
          return b.upstreamCalls - a.upstreamCalls
            || b.requests - a.requests
            || a.operation.localeCompare(b.operation)
        })
        .slice(0, limit)
        .map(buildRouteSummary),
      users: [...users.values()]
        .sort((a, b) => {
          const leftPct = a.lowestRemainingPct ?? Number.POSITIVE_INFINITY
          const rightPct = b.lowestRemainingPct ?? Number.POSITIVE_INFINITY

          return leftPct - rightPct
            || b.upstreamCalls - a.upstreamCalls
            || b.requests - a.requests
        })
        .slice(0, limit)
        .map(user => ({
          userId: user.userId,
          requests: user.requests,
          upstreamCalls: user.upstreamCalls,
          nearLimitEvents: user.nearLimitEvents,
          lowestRemainingPct: user.lowestRemainingPct,
          lastOperation: user.lastOperation,
          lastSeenAt: user.lastSeenAt,
        })),
      currentRateLimits: currentRateLimits.slice(0, limit),
    }
  }

  getOperationDrilldown(query: GithubCacheMetricsOperationDrilldownQuery): GithubCacheMetricsOperationDrilldown {
    const now = query.now ?? this.now()
    const windowMs = Math.max(query.windowMs ?? DEFAULT_OVERVIEW_WINDOW_MS, this.bucketMs)
    const from = now - windowMs

    this.prune(now)

    const buckets = [...this.buckets.values()]
      .filter(bucket => bucket.bucketStart >= from && bucket.bucketStart <= now)
      .sort((a, b) => a.bucketStart - b.bucketStart)

    let summaryAggregate: GithubMetricsOperationAggregate | null = null
    const series: GithubCacheMetricsOperationSeriesPoint[] = []

    for (const bucket of buckets) {
      let bucketAggregate: GithubMetricsOperationAggregate | null = null

      for (const aggregate of bucket.operations.values()) {
        if (!matchesOperationFilter(aggregate, query)) {
          continue
        }

        if (!bucketAggregate) {
          bucketAggregate = { ...aggregate }
        }
        else {
          mergeOperationAggregate(bucketAggregate, aggregate)
          bucketAggregate.scope = query.scope ?? bucketAggregate.scope
        }
      }

      if (!bucketAggregate) {
        continue
      }

      if (!summaryAggregate) {
        summaryAggregate = { ...bucketAggregate }
      }
      else {
        mergeOperationAggregate(summaryAggregate, bucketAggregate)
      }

      series.push(buildOperationSeriesPoint(bucket.bucketStart, bucketAggregate))
    }

    if (summaryAggregate && query.scope == null) {
      summaryAggregate.scope = undefined
    }

    return {
      from,
      to: now,
      bucketMs: this.bucketMs,
      selection: {
        operation: query.operation,
        scope: query.scope ?? null,
      },
      summary: summaryAggregate ? buildRouteSummary(summaryAggregate) : null,
      series,
    }
  }

  private getOrCreateBucket(at: number) {
    this.prune(at)

    const bucketStart = this.getBucketStart(at)
    const current = this.buckets.get(bucketStart)
    if (current) {
      return current
    }

    const nextBucket: GithubMetricsBucket = {
      bucketStart,
      summary: createEmptyCounters(),
      operations: new Map(),
      users: new Map(),
      resources: new Map(),
    }

    this.buckets.set(bucketStart, nextBucket)
    return nextBucket
  }

  private getBucketStart(at: number) {
    return Math.floor(at / this.bucketMs) * this.bucketMs
  }

  private getOrCreateOperation(
    bucket: GithubMetricsBucket,
    operation: string,
    scope: GithubCacheScope | undefined,
    at: number,
  ) {
    const operationKey = buildOperationAggregateKey(operation, scope)
    const current = bucket.operations.get(operationKey)
    if (current) {
      current.lastSeenAt = at
      return current
    }

    const nextOperation: GithubMetricsOperationAggregate = {
      operation,
      scope,
      ...createEmptyCounters(),
      paginatedLoads: 0,
      totalPageCount: 0,
      totalItemCount: 0,
      truncatedCount: 0,
      totalPaginationDurationMs: 0,
      lastSeenAt: at,
    }
    bucket.operations.set(operationKey, nextOperation)
    return nextOperation
  }

  private getOrCreateUser(bucket: GithubMetricsBucket, userId: string, at: number) {
    const current = bucket.users.get(userId)
    if (current) {
      current.lastSeenAt = at
      return current
    }

    const nextUser: GithubMetricsUserAggregate = {
      userId,
      requests: 0,
      upstreamCalls: 0,
      nearLimitEvents: 0,
      lowestRemainingPct: null,
      lastOperation: null,
      lastSeenAt: at,
    }

    bucket.users.set(userId, nextUser)
    return nextUser
  }

  private getOrCreateResource(bucket: GithubMetricsBucket, resource: string) {
    const current = bucket.resources.get(resource)
    if (current) {
      return current
    }

    const nextResource: GithubMetricsResourceAggregate = {
      resource,
      upstreamCalls: 0,
      notModified: 0,
      errors: 0,
      nearLimitEvents: 0,
    }

    bucket.resources.set(resource, nextResource)
    return nextResource
  }

  private getOrCreatePendingOperation(
    bucketStart: number,
    operation: string,
    scope: GithubCacheScope,
    at: number,
  ) {
    const operationKey = `${bucketStart}:${buildOperationAggregateKey(operation, scope)}`
    const current = this.pendingOperationMetrics.get(operationKey)
    if (current) {
      current.lastSeenAt = at
      return current
    }

    const nextOperation: GithubPersistedOperationMetric = {
      bucketStart,
      operation,
      scope,
      requests: 0,
      hits: 0,
      staleHits: 0,
      misses: 0,
      upstreamCalls: 0,
      notModified: 0,
      errorCount: 0,
      nearLimitEvents: 0,
      totalBackendDurationMs: 0,
      totalGithubDurationMs: 0,
      paginatedLoads: 0,
      totalPageCount: 0,
      totalItemCount: 0,
      truncatedCount: 0,
      totalPaginationDurationMs: 0,
      ttlMs: null,
      staleMs: null,
      lastSeenAt: at,
    }

    this.pendingOperationMetrics.set(operationKey, nextOperation)
    return nextOperation
  }

  private getOrCreatePendingResource(bucketStart: number, resource: string) {
    const resourceKey = `${bucketStart}:${resource}`
    const current = this.pendingResourceMetrics.get(resourceKey)
    if (current) {
      return current
    }

    const nextResource: GithubPersistedResourceMetric = {
      bucketStart,
      resource,
      upstreamCalls: 0,
      notModified: 0,
      errorCount: 0,
      nearLimitEvents: 0,
    }

    this.pendingResourceMetrics.set(resourceKey, nextResource)
    return nextResource
  }

  private getOrCreatePendingUser(bucketStart: number, userId: string, at: number) {
    const userKey = `${bucketStart}:${userId}`
    const current = this.pendingUserMetrics.get(userKey)
    if (current) {
      current.lastSeenAt = at
      return current
    }

    const nextUser: GithubPersistedUserMetric = {
      bucketStart,
      userId,
      requests: 0,
      upstreamCalls: 0,
      nearLimitEvents: 0,
      lowestRemainingPct: null,
      lastOperation: null,
      lastSeenAt: at,
    }

    this.pendingUserMetrics.set(userKey, nextUser)
    return nextUser
  }

  private prune(now: number) {
    const minimumBucketStart = Math.floor((now - this.retentionMs) / this.bucketMs) * this.bucketMs

    for (const bucketStart of this.buckets.keys()) {
      if (bucketStart < minimumBucketStart) {
        this.buckets.delete(bucketStart)
      }
    }

    for (const [key, rateLimit] of this.currentRateLimits.entries()) {
      if (rateLimit.updatedAt < now - this.retentionMs) {
        this.currentRateLimits.delete(key)
      }
    }
  }
}

export const githubMetricsCollector = createGithubMetricsCollector()
