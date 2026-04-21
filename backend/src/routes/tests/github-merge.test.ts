import type {
  GithubPullRequestMergeReadiness,
  GithubPullRequestMergeResult,
} from '../../plugins/github/types.js'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetchGithubPullRequestMergeReadiness = vi.fn()
const mergeGithubPullRequest = vi.fn()
const invalidateTags = vi.fn()
const getGithubPullRequestMutationTags = vi.fn()

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

vi.mock('../../plugins/github/pull-request-merge.js', () => ({
  fetchGithubPullRequestMergeReadiness,
}))

vi.mock('../../plugins/github/service.js', async () => {
  const actual = await vi.importActual<typeof import('../../plugins/github/service.js')>(
    '../../plugins/github/service.js',
  )

  return {
    ...actual,
    mergeGithubPullRequest,
  }
})

vi.mock('../../plugins/github/cache/github-cache-runtime.js', () => ({
  githubCache: {
    invalidateTags,
  },
}))

vi.mock('../../plugins/github/cache/github-repository-visibility-runtime.js', () => ({
  githubRepositoryVisibility: {
    isKnownPublic: vi.fn(),
    clear: vi.fn(),
    markPublic: vi.fn(),
  },
}))

vi.mock('../../plugins/github/cache/github-cache-policy.js', async () => {
  const actual = await vi.importActual<typeof import('../../plugins/github/cache/github-cache-policy.js')>(
    '../../plugins/github/cache/github-cache-policy.js',
  )

  return {
    ...actual,
    getGithubPullRequestMutationTags,
  }
})

vi.mock('../../plugins/github/metrics/github-metrics-context.js', () => ({
  runWithGithubMetricsContext: (_context: unknown, callback: () => Promise<unknown>) => callback(),
}))

function makeMergeReadiness(
  overrides: Partial<GithubPullRequestMergeReadiness> = {},
): GithubPullRequestMergeReadiness {
  return {
    status: 'ready',
    message: 'This pull request is ready to merge.',
    current_head_sha: 'head-sha',
    available_methods: ['merge', 'squash', 'rebase'],
    default_method: 'merge',
    can_merge_now: true,
    viewer_can_merge: true,
    mergeable_state: 'clean',
    rebaseable: true,
    auto_merge_enabled: false,
    auto_merge: null,
    viewer_can_enable_auto_merge: false,
    viewer_can_disable_auto_merge: false,
    ...overrides,
  }
}

function makeMergeResult(
  overrides: Partial<GithubPullRequestMergeResult> = {},
): GithubPullRequestMergeResult {
  return {
    merged: true,
    sha: 'merged-sha',
    message: 'Pull Request successfully merged',
    method: 'merge',
    ...overrides,
  }
}

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github merge routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    invalidateTags.mockResolvedValue(undefined)
    getGithubPullRequestMutationTags.mockReturnValue(['pr-tag', 'repo-tag'])
  })

  it('returns merge readiness using the current GitHub token and pull request params', async () => {
    fetchGithubPullRequestMergeReadiness.mockResolvedValue(
      makeMergeReadiness({
        status: 'blocked',
        message: 'This pull request is blocked by required reviews, checks, or repository rules.',
        can_merge_now: false,
      }),
    )

    const response = await request('/pr/42/merge-readiness?org=acme&repo=widget')

    expect(response.status).toBe(200)
    expect(fetchGithubPullRequestMergeReadiness).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        pull_number: 42,
      },
    })
    await expect(response.json()).resolves.toEqual({
      mergeReadiness: expect.objectContaining({
        status: 'blocked',
        can_merge_now: false,
      }),
    })
  })

  it.each([403, 404, 422])(
    'passes through merge readiness status %i from GitHub',
    async (status) => {
      fetchGithubPullRequestMergeReadiness.mockRejectedValue(
        Object.assign(new Error(`GitHub readiness error ${status}`), { status }),
      )

      const response = await request('/pr/42/merge-readiness?org=acme&repo=widget')

      expect(response.status).toBe(status)
      await expect(response.json()).resolves.toEqual({
        error: `GitHub readiness error ${status}`,
      })
    },
  )

  it('forwards merge params to GitHub, trims optional commit fields, and invalidates pull request tags', async () => {
    mergeGithubPullRequest.mockResolvedValue(
      makeMergeResult({
        method: 'squash',
      }),
    )

    const response = await request('/pr/42/merge?org=acme&repo=widget', {
      method: 'PUT',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        method: 'squash',
        expectedHeadSha: '  head-sha  ',
        commitTitle: '  Ship it  ',
        commitMessage: '  Merge summary  ',
      }),
    })

    expect(response.status).toBe(200)
    expect(mergeGithubPullRequest).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        pull_number: 42,
        sha: 'head-sha',
        merge_method: 'squash',
        commit_title: 'Ship it',
        commit_message: 'Merge summary',
      },
    })
    expect(getGithubPullRequestMutationTags).toHaveBeenCalledWith({
      userId: 'user-1',
      owner: 'acme',
      repo: 'widget',
      pullNumber: 42,
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
    await expect(response.json()).resolves.toEqual({
      mergeResult: {
        merged: true,
        sha: 'merged-sha',
        message: 'Pull Request successfully merged',
        method: 'squash',
      },
    })
  })

  it.each([403, 404, 405, 409, 422])(
    'passes through merge status %i from GitHub without invalidating cache',
    async (status) => {
      mergeGithubPullRequest.mockRejectedValue(
        Object.assign(new Error(`GitHub merge error ${status}`), { status }),
      )

      const response = await request('/pr/42/merge?org=acme&repo=widget', {
        method: 'PUT',
        headers: {
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          method: 'merge',
          expectedHeadSha: 'head-sha',
        }),
      })

      expect(response.status).toBe(status)
      expect(invalidateTags).not.toHaveBeenCalled()
      await expect(response.json()).resolves.toEqual({
        error: `GitHub merge error ${status}`,
      })
    },
  )
})
