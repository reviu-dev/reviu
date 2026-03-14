import { describe, expect, it } from 'vitest'
import { createGithubMetricsCollector } from './github-metrics.js'

describe('github metrics collector', () => {
  it('aggregates cache, upstream, and near-limit data into an overview', () => {
    const collector = createGithubMetricsCollector({
      now: () => 120_500,
    })

    collector.recordCacheEvent({
      at: 60_000,
      userId: 'user-1',
      operation: 'pull_request.details',
      scope: 'viewer',
      cacheStatus: 'miss',
      ttlMs: 20_000,
      staleMs: 120_000,
      durationMs: 18,
    })

    collector.recordGithubApiEvent({
      at: 60_050,
      userId: 'user-1',
      operation: 'pull_request.details',
      scope: 'viewer',
      route: 'GET /repos/{owner}/{repo}/pulls/{pull_number}',
      status: 200,
      durationMs: 42,
      rateLimit: {
        limit: 5_000,
        remaining: 4_200,
        resource: 'core',
      },
    })

    collector.recordCacheEvent({
      at: 60_100,
      userId: 'user-1',
      operation: 'pull_request.details',
      scope: 'viewer',
      cacheStatus: 'hit',
      ttlMs: 20_000,
      staleMs: 120_000,
      durationMs: 4,
    })

    collector.recordCacheEvent({
      at: 120_000,
      userId: 'user-2',
      operation: 'viewer.pull_requests.need_review',
      scope: 'viewer',
      cacheStatus: 'stale',
      ttlMs: 60_000,
      staleMs: 300_000,
      durationMs: 6,
    })

    collector.recordGithubApiEvent({
      at: 120_100,
      userId: 'user-2',
      operation: 'viewer.pull_requests.need_review',
      scope: 'viewer',
      route: 'GET /search/issues',
      status: 304,
      durationMs: 11,
      notModified: true,
      rateLimit: {
        limit: 30,
        remaining: 2,
        used: 28,
        reset: 1_800,
        resource: 'search',
      },
    })

    const overview = collector.getOverview({
      now: 120_500,
      windowMs: 5 * 60_000,
      limit: 10,
    })

    expect(overview.summary).toEqual({
      requests: 3,
      hits: 1,
      staleHits: 1,
      misses: 1,
      hitRate: 1 / 3,
      staleRate: 1 / 3,
      missRate: 1 / 3,
      upstreamCalls: 2,
      githubCallsSaved: 1,
      notModified: 1,
      errorCount: 0,
      nearLimitEvents: 1,
      usersNearLimit: 1,
      paginatedLoads: 0,
      avgPageCount: null,
      avgItemCount: null,
      truncatedCount: 0,
      avgPaginationDurationMs: null,
    })

    expect(overview.scopeSummary).toEqual([
      {
        scope: 'viewer',
        requests: 3,
        hits: 1,
        staleHits: 1,
        misses: 1,
        hitRate: 1 / 3,
        staleRate: 1 / 3,
        missRate: 1 / 3,
        upstreamCalls: 2,
        githubCallsSaved: 1,
        notModified: 1,
        notModifiedRate: 1 / 2,
        errorCount: 0,
        nearLimitEvents: 1,
        avgBackendDurationMs: (18 + 4 + 6) / 3,
        avgGithubDurationMs: (42 + 11) / 2,
        paginatedLoads: 0,
        avgPageCount: null,
        avgItemCount: null,
        truncatedCount: 0,
        avgPaginationDurationMs: null,
      },
    ])

    expect(overview.cacheStatusSeries).toEqual([
      {
        bucketStart: 60_000,
        hit: 1,
        stale: 0,
        miss: 1,
        upstreamCalls: 1,
        notModified: 0,
        errors: 0,
      },
      {
        bucketStart: 120_000,
        hit: 0,
        stale: 1,
        miss: 0,
        upstreamCalls: 1,
        notModified: 1,
        errors: 0,
      },
    ])

    expect(overview.githubResourceSeries).toEqual([
      {
        bucketStart: 60_000,
        resource: 'core',
        upstreamCalls: 1,
        notModified: 0,
        errors: 0,
        nearLimitEvents: 0,
      },
      {
        bucketStart: 120_000,
        resource: 'search',
        upstreamCalls: 1,
        notModified: 1,
        errors: 0,
        nearLimitEvents: 1,
      },
    ])

    expect(overview.routes).toEqual([
      expect.objectContaining({
        operation: 'pull_request.details',
        scope: 'viewer',
        requests: 2,
        hits: 1,
        staleHits: 0,
        misses: 1,
        upstreamCalls: 1,
        notModified: 0,
        ttlMs: 20_000,
        staleMs: 120_000,
      }),
      expect.objectContaining({
        operation: 'viewer.pull_requests.need_review',
        scope: 'viewer',
        requests: 1,
        hits: 0,
        staleHits: 1,
        misses: 0,
        upstreamCalls: 1,
        notModified: 1,
        ttlMs: 60_000,
        staleMs: 300_000,
      }),
    ])

    expect(overview.users).toEqual([
      {
        userId: 'user-2',
        requests: 1,
        upstreamCalls: 1,
        nearLimitEvents: 1,
        lowestRemainingPct: 2 / 30,
        lastOperation: 'viewer.pull_requests.need_review',
        lastSeenAt: 120_100,
      },
      {
        userId: 'user-1',
        requests: 2,
        upstreamCalls: 1,
        nearLimitEvents: 0,
        lowestRemainingPct: 4_200 / 5_000,
        lastOperation: 'pull_request.details',
        lastSeenAt: 60_100,
      },
    ])

    expect(overview.currentRateLimits).toEqual([
      {
        userId: 'user-2',
        resource: 'search',
        remaining: 2,
        limit: 30,
        used: 28,
        reset: 1_800,
        remainingPct: 2 / 30,
        lastOperation: 'viewer.pull_requests.need_review',
        lastRoute: 'GET /search/issues',
        lastStatus: 304,
        updatedAt: 120_100,
      },
      {
        userId: 'user-1',
        resource: 'core',
        remaining: 4_200,
        limit: 5_000,
        used: null,
        reset: null,
        remainingPct: 4_200 / 5_000,
        lastOperation: 'pull_request.details',
        lastRoute: 'GET /repos/{owner}/{repo}/pulls/{pull_number}',
        lastStatus: 200,
        updatedAt: 60_050,
      },
    ])
  })

  it('keeps viewer and public aggregates separate for the same operation', () => {
    const collector = createGithubMetricsCollector({
      now: () => 120_000,
    })

    collector.recordCacheEvent({
      at: 60_000,
      userId: 'user-1',
      operation: 'repository.readme',
      scope: 'viewer',
      cacheStatus: 'miss',
      ttlMs: 120_000,
      staleMs: 600_000,
      durationMs: 12,
    })

    collector.recordGithubApiEvent({
      at: 60_050,
      userId: 'user-1',
      operation: 'repository.readme',
      scope: 'viewer',
      route: 'GET /repos/{owner}/{repo}/readme',
      status: 200,
      durationMs: 30,
      rateLimit: {
        limit: 5_000,
        remaining: 4_000,
        resource: 'core',
      },
    })

    collector.recordCacheEvent({
      at: 60_100,
      userId: 'user-2',
      operation: 'repository.readme',
      scope: 'public',
      cacheStatus: 'hit',
      ttlMs: 120_000,
      staleMs: 600_000,
      durationMs: 3,
    })

    const overview = collector.getOverview({
      now: 120_000,
      windowMs: 5 * 60_000,
      limit: 10,
    })

    expect(overview.routes).toEqual(expect.arrayContaining([
      expect.objectContaining({
        operation: 'repository.readme',
        scope: 'viewer',
        requests: 1,
        hits: 0,
        misses: 1,
        upstreamCalls: 1,
      }),
      expect.objectContaining({
        operation: 'repository.readme',
        scope: 'public',
        requests: 1,
        hits: 1,
        misses: 0,
        upstreamCalls: 0,
      }),
    ]))

    expect(overview.scopeSummary).toEqual([
      {
        scope: 'public',
        requests: 1,
        hits: 1,
        staleHits: 0,
        misses: 0,
        hitRate: 1,
        staleRate: 0,
        missRate: 0,
        upstreamCalls: 0,
        githubCallsSaved: 1,
        notModified: 0,
        notModifiedRate: 0,
        errorCount: 0,
        nearLimitEvents: 0,
        avgBackendDurationMs: 3,
        avgGithubDurationMs: null,
        paginatedLoads: 0,
        avgPageCount: null,
        avgItemCount: null,
        truncatedCount: 0,
        avgPaginationDurationMs: null,
      },
      {
        scope: 'viewer',
        requests: 1,
        hits: 0,
        staleHits: 0,
        misses: 1,
        hitRate: 0,
        staleRate: 0,
        missRate: 1,
        upstreamCalls: 1,
        githubCallsSaved: 0,
        notModified: 0,
        notModifiedRate: 0,
        errorCount: 0,
        nearLimitEvents: 0,
        avgBackendDurationMs: 12,
        avgGithubDurationMs: 30,
        paginatedLoads: 0,
        avgPageCount: null,
        avgItemCount: null,
        truncatedCount: 0,
        avgPaginationDurationMs: null,
      },
    ])
  })

  it('aggregates pagination metrics into summary, scope, and routes', () => {
    const collector = createGithubMetricsCollector({
      now: () => 180_000,
    })

    collector.recordPaginationEvent({
      at: 120_000,
      userId: 'user-1',
      operation: 'pull_request.comments',
      scope: 'viewer',
      pageCount: 3,
      itemCount: 240,
      truncated: true,
      durationMs: 120,
    })

    collector.recordPaginationEvent({
      at: 120_500,
      userId: 'user-1',
      operation: 'pull_request.comments',
      scope: 'viewer',
      pageCount: 2,
      itemCount: 120,
      truncated: false,
      durationMs: 80,
    })

    const overview = collector.getOverview({
      now: 180_000,
      windowMs: 5 * 60_000,
      limit: 10,
    })

    expect(overview.summary).toEqual(expect.objectContaining({
      paginatedLoads: 2,
      avgPageCount: 2.5,
      avgItemCount: 180,
      truncatedCount: 1,
      avgPaginationDurationMs: 100,
    }))

    expect(overview.scopeSummary).toEqual([
      expect.objectContaining({
        scope: 'viewer',
        paginatedLoads: 2,
        avgPageCount: 2.5,
        avgItemCount: 180,
        truncatedCount: 1,
        avgPaginationDurationMs: 100,
      }),
    ])

    expect(overview.routes).toEqual([
      expect.objectContaining({
        operation: 'pull_request.comments',
        scope: 'viewer',
        paginatedLoads: 2,
        avgPageCount: 2.5,
        avgItemCount: 180,
        truncatedCount: 1,
        avgPaginationDurationMs: 100,
      }),
    ])
  })

  it('builds an operation drilldown series for a selected route', () => {
    const collector = createGithubMetricsCollector({
      now: () => 180_000,
    })

    collector.recordCacheEvent({
      at: 120_000,
      userId: 'user-1',
      operation: 'pull_request.comments',
      scope: 'viewer',
      cacheStatus: 'miss',
      ttlMs: 15_000,
      staleMs: 120_000,
      durationMs: 20,
    })

    collector.recordGithubApiEvent({
      at: 120_050,
      userId: 'user-1',
      operation: 'pull_request.comments',
      scope: 'viewer',
      route: 'GET /repos/{owner}/{repo}/pulls/{pull_number}/comments',
      status: 200,
      durationMs: 45,
      rateLimit: {
        limit: 5_000,
        remaining: 4_100,
        resource: 'core',
      },
    })

    collector.recordPaginationEvent({
      at: 120_070,
      userId: 'user-1',
      operation: 'pull_request.comments',
      scope: 'viewer',
      pageCount: 3,
      itemCount: 220,
      truncated: true,
      durationMs: 90,
    })

    collector.recordCacheEvent({
      at: 180_000,
      userId: 'user-1',
      operation: 'pull_request.comments',
      scope: 'viewer',
      cacheStatus: 'hit',
      ttlMs: 15_000,
      staleMs: 120_000,
      durationMs: 5,
    })

    const drilldown = collector.getOperationDrilldown({
      now: 180_000,
      windowMs: 5 * 60_000,
      operation: 'pull_request.comments',
      scope: 'viewer',
    })

    expect(drilldown.selection).toEqual({
      operation: 'pull_request.comments',
      scope: 'viewer',
    })

    expect(drilldown.summary).toEqual(expect.objectContaining({
      operation: 'pull_request.comments',
      scope: 'viewer',
      requests: 2,
      hits: 1,
      misses: 1,
      upstreamCalls: 1,
      paginatedLoads: 1,
      avgPageCount: 3,
      avgItemCount: 220,
      truncatedCount: 1,
      avgPaginationDurationMs: 90,
      ttlMs: 15_000,
      staleMs: 120_000,
    }))

    expect(drilldown.series).toEqual([
      expect.objectContaining({
        bucketStart: 120_000,
        requests: 1,
        hit: 0,
        miss: 1,
        upstreamCalls: 1,
        paginatedLoads: 1,
        avgPageCount: 3,
        avgItemCount: 220,
        truncatedCount: 1,
      }),
      expect.objectContaining({
        bucketStart: 180_000,
        requests: 1,
        hit: 1,
        miss: 0,
        upstreamCalls: 0,
        paginatedLoads: 0,
        avgPageCount: null,
        truncatedCount: 0,
      }),
    ])
  })

  it('drains persisted metrics without losing the live in-memory overview', () => {
    const collector = createGithubMetricsCollector({
      now: () => 60_000,
    })

    collector.recordCacheEvent({
      at: 60_000,
      userId: 'user-1',
      operation: 'repository.readme',
      scope: 'public',
      cacheStatus: 'hit',
      ttlMs: 120_000,
      staleMs: 600_000,
      durationMs: 5,
    })

    const snapshot = collector.drainPersistedMetrics()

    expect(snapshot.operationMetrics).toEqual([
      {
        bucketStart: 60_000,
        operation: 'repository.readme',
        scope: 'public',
        requests: 1,
        hits: 1,
        staleHits: 0,
        misses: 0,
        upstreamCalls: 0,
        notModified: 0,
        errorCount: 0,
        nearLimitEvents: 0,
        totalBackendDurationMs: 5,
        totalGithubDurationMs: 0,
        paginatedLoads: 0,
        totalPageCount: 0,
        totalItemCount: 0,
        truncatedCount: 0,
        totalPaginationDurationMs: 0,
        ttlMs: 120_000,
        staleMs: 600_000,
        lastSeenAt: 60_000,
      },
    ])

    expect(collector.drainPersistedMetrics()).toEqual({
      bucketMs: 60_000,
      operationMetrics: [],
      resourceMetrics: [],
      userMetrics: [],
      rateLimitStates: [],
    })

    const overview = collector.getOverview({
      now: 60_000,
      windowMs: 5 * 60_000,
      limit: 10,
    })

    expect(overview.summary.requests).toBe(1)
    expect(overview.scopeSummary).toEqual([
      expect.objectContaining({
        scope: 'public',
        requests: 1,
        hits: 1,
      }),
    ])
  })
})
