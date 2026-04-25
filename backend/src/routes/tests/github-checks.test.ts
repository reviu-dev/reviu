import type { GithubPullRequestChecksSummary } from '../../plugins/github/types.js'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetchGithubPullRequestChecksSummary = vi.fn()

vi.mock('../../middlewares/auth.js', async () => {
  const { createMiddleware } = await import('hono/factory')

  return {
    authMiddlewarePro: createMiddleware(async (ctx, next) => {
      ctx.set('user', {
        id: 'user-1',
        createdAt: new Date('2026-03-19T00:00:00Z'),
        updatedAt: new Date('2026-03-19T00:00:00Z'),
        email: 'user@example.com',
        emailVerified: true,
        name: 'Reviu Test User',
        image: null,
        proGrantedAt: new Date('2026-03-19T00:00:00Z'),
        role: 'user',
        banned: false,
        banReason: null,
        banExpires: null,
        github: {
          accessToken: 'github-token',
          accessTokenExpiresAt: undefined,
          scopes: ['repo'],
          idToken: undefined,
        },
      } as any)
      await next()
    }),
  }
})

vi.mock('../../plugins/github/pull-request-checks.js', () => ({
  fetchGithubPullRequestChecksSummary,
}))

vi.mock('../../plugins/github/cache/github-cache-runtime.js', () => ({
  githubCache: {
    getOrLoad: vi.fn(),
    invalidateTags: vi.fn(),
    prime: vi.fn(),
  },
}))

vi.mock('../../plugins/github/cache/github-repository-visibility-runtime.js', () => ({
  githubRepositoryVisibility: {
    isKnownPublic: vi.fn(),
    clear: vi.fn(),
    markPublic: vi.fn(),
  },
}))

vi.mock('../../plugins/github/metrics/github-metrics-context.js', () => ({
  runWithGithubMetricsContext: (_context: unknown, callback: () => Promise<unknown>) => callback(),
}))

function makeChecksSummary(
  overrides: Partial<GithubPullRequestChecksSummary> = {},
): GithubPullRequestChecksSummary {
  return {
    head_sha: 'head-sha',
    overall_state: 'failure',
    required_state: 'failure',
    total_checks: 3,
    successful_checks: 1,
    failed_checks: 1,
    pending_checks: 1,
    skipped_checks: 0,
    required_checks_total: 2,
    required_checks_passed: 1,
    required_checks_failed: 1,
    required_checks_pending: 0,
    required_checks_skipped: 0,
    required_contexts: ['build', 'lint'],
    missing_required_contexts: [],
    requires_up_to_date_branch: true,
    actions_runs: [],
    other_checks: [],
    legacy_statuses: [],
    ...overrides,
  }
}

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github checks routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns pull request checks using the current GitHub token and pull request params', async () => {
    fetchGithubPullRequestChecksSummary.mockResolvedValue(makeChecksSummary())

    const response = await request('/pr/42/checks?org=acme&repo=widget')

    expect(response.status).toBe(200)
    expect(fetchGithubPullRequestChecksSummary).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        pull_number: 42,
      },
    })
    await expect(response.json()).resolves.toEqual({
      checks: expect.objectContaining({
        head_sha: 'head-sha',
        overall_state: 'failure',
      }),
    })
  })

  it.each([403, 404, 422])(
    'passes through checks status %i from GitHub',
    async (status) => {
      fetchGithubPullRequestChecksSummary.mockRejectedValue(
        Object.assign(new Error(`GitHub checks error ${status}`), { status }),
      )

      const response = await request('/pr/42/checks?org=acme&repo=widget')

      expect(response.status).toBe(status)
      await expect(response.json()).resolves.toEqual({
        error: `GitHub checks error ${status}`,
      })
    },
  )
})
