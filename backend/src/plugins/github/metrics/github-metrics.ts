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

export interface GithubCacheMetricsOverviewQuery {
  now?: number
  windowMs?: number
  limit?: number
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
  routes: Array<{
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
    ttlMs: number | null
    staleMs: number | null
    lastSeenAt: number
  }>
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

  constructor(
    private readonly now: () => number,
    private readonly bucketMs: number,
    private readonly retentionMs: number,
  ) {}

  recordCacheEvent(event: GithubCacheMetricEvent) {
    const at = event.at ?? this.now()
    const bucket = this.getOrCreateBucket(at)
    const operation = this.getOrCreateOperation(bucket, event.operation, event.scope, at)

    bucket.summary.requests += 1
    bucket.summary.totalBackendDurationMs += event.durationMs

    operation.requests += 1
    operation.totalBackendDurationMs += event.durationMs
    operation.scope = event.scope
    operation.ttlMs = event.ttlMs
    operation.staleMs = event.staleMs
    operation.lastSeenAt = at

    if (event.cacheStatus === 'hit') {
      bucket.summary.hit += 1
      operation.hit += 1
    }
    else if (event.cacheStatus === 'stale') {
      bucket.summary.stale += 1
      operation.stale += 1
    }
    else {
      bucket.summary.miss += 1
      operation.miss += 1
    }

    if (event.userId) {
      const user = this.getOrCreateUser(bucket, event.userId, at)
      user.requests += 1
      user.lastOperation = event.operation
      user.lastSeenAt = at
    }
  }

  recordGithubApiEvent(event: GithubApiMetricEvent) {
    const at = event.at ?? this.now()
    const bucket = this.getOrCreateBucket(at)
    const operation = this.getOrCreateOperation(bucket, event.operation, event.scope, at)
    const resource = event.rateLimit?.resource ?? 'unknown'
    const resourceAggregate = this.getOrCreateResource(bucket, resource)
    const nearLimit = isNearLimit(event.rateLimit)
    const error = event.status >= 400 && event.status !== 304

    bucket.summary.upstreamCalls += 1
    bucket.summary.totalGithubDurationMs += event.durationMs

    operation.upstreamCalls += 1
    operation.totalGithubDurationMs += event.durationMs
    operation.lastSeenAt = at

    resourceAggregate.upstreamCalls += 1

    if (event.notModified) {
      bucket.summary.notModified += 1
      operation.notModified += 1
      resourceAggregate.notModified += 1
    }

    if (error) {
      bucket.summary.errors += 1
      operation.errors += 1
      resourceAggregate.errors += 1
    }

    if (nearLimit) {
      bucket.summary.nearLimitEvents += 1
      operation.nearLimitEvents += 1
      resourceAggregate.nearLimitEvents += 1
    }

    if (event.userId) {
      const user = this.getOrCreateUser(bucket, event.userId, at)
      user.upstreamCalls += 1
      user.lastOperation = event.operation
      user.lastSeenAt = at

      if (nearLimit) {
        user.nearLimitEvents += 1
      }

      const remainingPct = calculateRemainingPct(event.rateLimit)
      if (remainingPct != null && (user.lowestRemainingPct == null || remainingPct < user.lowestRemainingPct)) {
        user.lowestRemainingPct = remainingPct
      }

      if (event.rateLimit) {
        this.currentRateLimits.set(
          `${event.userId}:${resource}`,
          {
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
          },
        )
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
    const scopeSummary = new Map<GithubCacheScope, GithubMetricsCounters>()
    const users = new Map<string, GithubMetricsUserAggregate>()
    const resourceSeries: GithubCacheMetricsOverview['githubResourceSeries'] = []

    for (const bucket of buckets) {
      mergeCounters(totals, bucket.summary)

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

        mergeCounters(current, aggregate)
        current.scope = aggregate.scope ?? current.scope
        current.ttlMs = aggregate.ttlMs ?? current.ttlMs
        current.staleMs = aggregate.staleMs ?? current.staleMs
        current.lastSeenAt = Math.max(current.lastSeenAt, aggregate.lastSeenAt)
      }

      for (const aggregate of bucket.operations.values()) {
        if (!aggregate.scope) {
          continue
        }

        const current = scopeSummary.get(aggregate.scope) ?? createEmptyCounters()
        mergeCounters(current, aggregate)
        scopeSummary.set(aggregate.scope, current)
      }

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
        })),
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
        .map(operation => ({
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
          ttlMs: operation.ttlMs ?? null,
          staleMs: operation.staleMs ?? null,
          lastSeenAt: operation.lastSeenAt,
        })),
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

  private getOrCreateBucket(at: number) {
    this.prune(at)

    const bucketStart = Math.floor(at / this.bucketMs) * this.bucketMs
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
