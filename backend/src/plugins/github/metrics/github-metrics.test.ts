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
    })

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
})
