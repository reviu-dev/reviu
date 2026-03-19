import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

const { executeMock } = vi.hoisted(() => ({
  executeMock: vi.fn(),
}))

vi.mock('../../../db/index.js', () => ({
  db: {
    execute: executeMock,
  },
}))

let buildGithubMetricsPruneCutoffs: typeof import('./github-metrics-store.js').buildGithubMetricsPruneCutoffs
let pruneGithubMetrics: typeof import('./github-metrics-store.js').pruneGithubMetrics

const DAY_MS = 24 * 60 * 60_000

beforeAll(async () => {
  ;({
    buildGithubMetricsPruneCutoffs,
    pruneGithubMetrics,
  } = await import('./github-metrics-store.js'))
})

afterEach(() => {
  executeMock.mockReset()
})

describe('github metrics prune', () => {
  it('builds retention cutoffs from the configured day windows', () => {
    const result = buildGithubMetricsPruneCutoffs({
      now: 100 * DAY_MS,
      metricsRetentionDays: 30,
      rateLimitStateRetentionDays: 14,
    })

    expect(result).toEqual({
      metricsCutoff: new Date(70 * DAY_MS),
      rateLimitStateCutoff: new Date(86 * DAY_MS),
      metricsRetentionDays: 30,
      rateLimitStateRetentionDays: 14,
    })
  })

  it('deletes old rows from every metrics table and returns deleted counts', async () => {
    executeMock
      .mockResolvedValueOnce({ rows: [{ count: 11 }] })
      .mockResolvedValueOnce({ rows: [{ count: 7 }] })
      .mockResolvedValueOnce({ rows: [{ count: 5 }] })
      .mockResolvedValueOnce({ rows: [{ count: 3 }] })

    const result = await pruneGithubMetrics({
      now: 100 * DAY_MS,
      metricsRetentionDays: 30,
      rateLimitStateRetentionDays: 14,
    })

    expect(executeMock).toHaveBeenCalledTimes(4)
    expect(result).toEqual({
      metricsCutoff: new Date(70 * DAY_MS),
      rateLimitStateCutoff: new Date(86 * DAY_MS),
      metricsRetentionDays: 30,
      rateLimitStateRetentionDays: 14,
      deletedOperationMetrics: 11,
      deletedResourceMetrics: 7,
      deletedUserMetrics: 5,
      deletedRateLimitStates: 3,
      totalDeleted: 26,
    })
  })
})
