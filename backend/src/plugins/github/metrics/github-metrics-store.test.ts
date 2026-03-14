import { beforeAll, describe, expect, it } from 'vitest'

let buildGithubMetricsOverviewFromPersistedRows: typeof import('./github-metrics-store.js').buildGithubMetricsOverviewFromPersistedRows
let buildGithubMetricsOperationDrilldownFromPersistedRows: typeof import('./github-metrics-store.js').buildGithubMetricsOperationDrilldownFromPersistedRows

beforeAll(async () => {
  process.env.NODE_ENV = 'development'
  process.env.BASE_URL ??= 'http://localhost:3000'
  process.env.PG_USER ??= 'postgres'
  process.env.PG_PASSWORD ??= 'postgres'
  process.env.PG_HOST ??= 'localhost'
  process.env.PG_PORT ??= '5432'
  process.env.PG_DATABASE ??= 'app'
  process.env.AUTH_SECRET ??= 'test-secret'
  process.env.GITHUB_OAUTH_CLIENT_SECRET ??= 'test-secret'
  process.env.GITHUB_OAUTH_CLIENT_ID ??= 'test-client-id'
  process.env.REDIS_HOST ??= 'localhost'
  process.env.REDIS_PORT ??= '6379'
  process.env.POLAR_ACCESS_TOKEN ??= 'polar-token'
  process.env.POLAR_SUCCESS_URL ??= 'http://localhost:3000/polar/success'
  process.env.POLAR_SUBSCRIPTION_PRODUCT_ID ??= 'product-id'
  process.env.DESKTOP_UPDATE_MANIFEST_URL ??= 'http://localhost:3000/desktop/updates'
  process.env.WEB_DASHBOARD_URL ??= 'http://localhost:5173'

  ;({
    buildGithubMetricsOverviewFromPersistedRows,
    buildGithubMetricsOperationDrilldownFromPersistedRows,
  } = await import('./github-metrics-store.js'))
})

describe('github metrics store', () => {
  it('builds an overview from persisted metric rows', () => {
    const overview = buildGithubMetricsOverviewFromPersistedRows({
      now: 180_000,
      windowMs: 120_000,
      limit: 10,
      operationMetrics: [
        {
          bucketStart: 0,
          operation: 'repository.readme',
          scope: 'public',
          requests: 99,
          hits: 99,
          staleHits: 0,
          misses: 0,
          upstreamCalls: 0,
          notModified: 0,
          errorCount: 0,
          nearLimitEvents: 0,
          totalBackendDurationMs: 99,
          totalGithubDurationMs: 0,
          paginatedLoads: 0,
          totalPageCount: 0,
          totalItemCount: 0,
          truncatedCount: 0,
          totalPaginationDurationMs: 0,
          ttlMs: 120_000,
          staleMs: 600_000,
          lastSeenAt: 1_000,
        },
        {
          bucketStart: 60_000,
          operation: 'repository.readme',
          scope: 'public',
          requests: 2,
          hits: 1,
          staleHits: 0,
          misses: 1,
          upstreamCalls: 1,
          notModified: 0,
          errorCount: 0,
          nearLimitEvents: 0,
          totalBackendDurationMs: 15,
          totalGithubDurationMs: 30,
          paginatedLoads: 0,
          totalPageCount: 0,
          totalItemCount: 0,
          truncatedCount: 0,
          totalPaginationDurationMs: 0,
          ttlMs: 120_000,
          staleMs: 600_000,
          lastSeenAt: 60_050,
        },
        {
          bucketStart: 120_000,
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
          totalBackendDurationMs: 4,
          totalGithubDurationMs: 0,
          paginatedLoads: 0,
          totalPageCount: 0,
          totalItemCount: 0,
          truncatedCount: 0,
          totalPaginationDurationMs: 0,
          ttlMs: 120_000,
          staleMs: 600_000,
          lastSeenAt: 120_010,
        },
        {
          bucketStart: 120_000,
          operation: 'viewer.notifications',
          scope: 'viewer',
          requests: 1,
          hits: 0,
          staleHits: 1,
          misses: 0,
          upstreamCalls: 1,
          notModified: 1,
          errorCount: 0,
          nearLimitEvents: 1,
          totalBackendDurationMs: 6,
          totalGithubDurationMs: 10,
          paginatedLoads: 0,
          totalPageCount: 0,
          totalItemCount: 0,
          truncatedCount: 0,
          totalPaginationDurationMs: 0,
          ttlMs: 15_000,
          staleMs: 60_000,
          lastSeenAt: 120_100,
        },
      ],
      resourceMetrics: [
        {
          bucketStart: 60_000,
          resource: 'core',
          upstreamCalls: 1,
          notModified: 0,
          errorCount: 0,
          nearLimitEvents: 0,
        },
        {
          bucketStart: 120_000,
          resource: 'search',
          upstreamCalls: 1,
          notModified: 1,
          errorCount: 0,
          nearLimitEvents: 1,
        },
      ],
      userMetrics: [
        {
          bucketStart: 60_000,
          userId: 'user-1',
          requests: 2,
          upstreamCalls: 1,
          nearLimitEvents: 0,
          lowestRemainingPct: 0.8,
          lastOperation: 'repository.readme',
          lastSeenAt: 60_050,
        },
        {
          bucketStart: 120_000,
          userId: 'user-1',
          requests: 1,
          upstreamCalls: 0,
          nearLimitEvents: 0,
          lowestRemainingPct: 0.7,
          lastOperation: 'repository.readme',
          lastSeenAt: 120_010,
        },
        {
          bucketStart: 120_000,
          userId: 'user-2',
          requests: 1,
          upstreamCalls: 1,
          nearLimitEvents: 1,
          lowestRemainingPct: 0.05,
          lastOperation: 'viewer.notifications',
          lastSeenAt: 120_100,
        },
      ],
      rateLimitStates: [
        {
          userId: 'user-1',
          resource: 'core',
          remaining: 4_000,
          limit: 5_000,
          used: 1_000,
          reset: 200_000,
          remainingPct: 0.8,
          lastOperation: 'repository.readme',
          lastRoute: 'GET /repos/{owner}/{repo}/readme',
          lastStatus: 200,
          updatedAt: 60_050,
        },
        {
          userId: 'user-2',
          resource: 'search',
          remaining: 1,
          limit: 20,
          used: 19,
          reset: 200_000,
          remainingPct: 0.05,
          lastOperation: 'viewer.notifications',
          lastRoute: 'GET /notifications',
          lastStatus: 304,
          updatedAt: 120_100,
        },
      ],
    })

    expect(overview.summary).toEqual({
      requests: 4,
      hits: 2,
      staleHits: 1,
      misses: 1,
      hitRate: 0.5,
      staleRate: 0.25,
      missRate: 0.25,
      upstreamCalls: 2,
      githubCallsSaved: 2,
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
        scope: 'public',
        requests: 3,
        hits: 2,
        staleHits: 0,
        misses: 1,
        hitRate: 2 / 3,
        staleRate: 0,
        missRate: 1 / 3,
        upstreamCalls: 1,
        githubCallsSaved: 2,
        notModified: 0,
        notModifiedRate: 0,
        errorCount: 0,
        nearLimitEvents: 0,
        avgBackendDurationMs: 19 / 3,
        avgGithubDurationMs: 30,
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
        staleHits: 1,
        misses: 0,
        hitRate: 0,
        staleRate: 1,
        missRate: 0,
        upstreamCalls: 1,
        githubCallsSaved: 0,
        notModified: 1,
        notModifiedRate: 1,
        errorCount: 0,
        nearLimitEvents: 1,
        avgBackendDurationMs: 6,
        avgGithubDurationMs: 10,
        paginatedLoads: 0,
        avgPageCount: null,
        avgItemCount: null,
        truncatedCount: 0,
        avgPaginationDurationMs: null,
      },
    ])

    expect(overview.scopeSeries).toEqual([
      {
        bucketStart: 60_000,
        scope: 'public',
        requests: 2,
        upstreamCalls: 1,
        githubCallsSaved: 1,
        hit: 1,
        stale: 0,
        miss: 1,
      },
      {
        bucketStart: 120_000,
        scope: 'public',
        requests: 1,
        upstreamCalls: 0,
        githubCallsSaved: 1,
        hit: 1,
        stale: 0,
        miss: 0,
      },
      {
        bucketStart: 120_000,
        scope: 'viewer',
        requests: 1,
        upstreamCalls: 1,
        githubCallsSaved: 0,
        hit: 0,
        stale: 1,
        miss: 0,
      },
    ])

    expect(overview.routes).toEqual([
      expect.objectContaining({
        operation: 'repository.readme',
        scope: 'public',
        requests: 3,
        hits: 2,
        staleHits: 0,
        misses: 1,
        upstreamCalls: 1,
        ttlMs: 120_000,
        staleMs: 600_000,
        lastSeenAt: 120_010,
      }),
      expect.objectContaining({
        operation: 'viewer.notifications',
        scope: 'viewer',
        requests: 1,
        hits: 0,
        staleHits: 1,
        misses: 0,
        upstreamCalls: 1,
        notModified: 1,
        nearLimitEvents: 1,
        ttlMs: 15_000,
        staleMs: 60_000,
        lastSeenAt: 120_100,
      }),
    ])

    expect(overview.users).toEqual([
      {
        userId: 'user-2',
        requests: 1,
        upstreamCalls: 1,
        nearLimitEvents: 1,
        lowestRemainingPct: 0.05,
        lastOperation: 'viewer.notifications',
        lastSeenAt: 120_100,
      },
      {
        userId: 'user-1',
        requests: 3,
        upstreamCalls: 1,
        nearLimitEvents: 0,
        lowestRemainingPct: 0.7,
        lastOperation: 'repository.readme',
        lastSeenAt: 120_010,
      },
    ])

    expect(overview.currentRateLimits).toEqual([
      expect.objectContaining({
        userId: 'user-2',
        resource: 'search',
        remainingPct: 0.05,
      }),
      expect.objectContaining({
        userId: 'user-1',
        resource: 'core',
        remainingPct: 0.8,
      }),
    ])
  })

  it('aggregates pagination metrics from persisted rows', () => {
    const overview = buildGithubMetricsOverviewFromPersistedRows({
      now: 180_000,
      windowMs: 120_000,
      limit: 10,
      operationMetrics: [
        {
          bucketStart: 120_000,
          operation: 'pull_request.comments',
          scope: 'viewer',
          requests: 2,
          hits: 0,
          staleHits: 0,
          misses: 2,
          upstreamCalls: 2,
          notModified: 0,
          errorCount: 0,
          nearLimitEvents: 0,
          totalBackendDurationMs: 40,
          totalGithubDurationMs: 160,
          paginatedLoads: 2,
          totalPageCount: 5,
          totalItemCount: 360,
          truncatedCount: 1,
          totalPaginationDurationMs: 200,
          ttlMs: 30_000,
          staleMs: 120_000,
          lastSeenAt: 120_100,
        },
      ],
      resourceMetrics: [],
      userMetrics: [],
      rateLimitStates: [],
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

  it('builds a persisted drilldown series for a selected operation', () => {
    const drilldown = buildGithubMetricsOperationDrilldownFromPersistedRows({
      now: 180_000,
      windowMs: 120_000,
      operation: 'pull_request.comments',
      scope: 'viewer',
      operationMetrics: [
        {
          bucketStart: 120_000,
          operation: 'pull_request.comments',
          scope: 'viewer',
          requests: 1,
          hits: 0,
          staleHits: 0,
          misses: 1,
          upstreamCalls: 1,
          notModified: 0,
          errorCount: 0,
          nearLimitEvents: 0,
          totalBackendDurationMs: 20,
          totalGithubDurationMs: 50,
          paginatedLoads: 1,
          totalPageCount: 3,
          totalItemCount: 240,
          truncatedCount: 1,
          totalPaginationDurationMs: 120,
          ttlMs: 15_000,
          staleMs: 120_000,
          lastSeenAt: 120_050,
        },
        {
          bucketStart: 180_000,
          operation: 'pull_request.comments',
          scope: 'viewer',
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
          ttlMs: 15_000,
          staleMs: 120_000,
          lastSeenAt: 180_000,
        },
      ],
      bucketMs: 60_000,
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
      avgItemCount: 240,
      truncatedCount: 1,
      avgPaginationDurationMs: 120,
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
        avgItemCount: 240,
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
})
