import { beforeEach, describe, expect, it, vi } from 'vitest'

const markGithubPullRequestReadyForReview = vi.fn()
const convertGithubPullRequestToDraft = vi.fn()
const invalidateTags = vi.fn()
const getGithubPullRequestMutationTags = vi.fn()

vi.mock('../middlewares/auth.js', async () => {
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

vi.mock('../plugins/github/service.js', async () => {
  const actual = await vi.importActual<typeof import('../plugins/github/service.js')>(
    '../plugins/github/service.js',
  )

  return {
    ...actual,
    markGithubPullRequestReadyForReview,
    convertGithubPullRequestToDraft,
  }
})

vi.mock('../plugins/github/cache/github-cache-runtime.js', () => ({
  githubCache: {
    invalidateTags,
  },
}))

vi.mock('../plugins/github/cache/github-repository-visibility-runtime.js', () => ({
  githubRepositoryVisibility: {
    isKnownPublic: vi.fn(),
    clear: vi.fn(),
    markPublic: vi.fn(),
  },
}))

vi.mock('../plugins/github/cache/github-cache-policy.js', async () => {
  const actual = await vi.importActual<typeof import('../plugins/github/cache/github-cache-policy.js')>(
    '../plugins/github/cache/github-cache-policy.js',
  )

  return {
    ...actual,
    getGithubPullRequestMutationTags,
  }
})

vi.mock('../plugins/github/metrics/github-metrics-context.js', () => ({
  runWithGithubMetricsContext: (_context: unknown, callback: () => Promise<unknown>) => callback(),
}))

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('./github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github pull request status routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    invalidateTags.mockResolvedValue(undefined)
    getGithubPullRequestMutationTags.mockReturnValue(['pr-tag', 'repo-tag'])
  })

  it('marks a draft pull request ready for review and invalidates pull request tags', async () => {
    markGithubPullRequestReadyForReview.mockResolvedValue(undefined)

    const response = await request('/pr/42/ready-for-review?org=acme&repo=widget', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        pullRequestId: 'PR_kwDOExample',
      }),
    })

    expect(response.status).toBe(204)
    expect(markGithubPullRequestReadyForReview).toHaveBeenCalledWith({
      token: 'github-token',
      pullRequestId: 'PR_kwDOExample',
    })
    expect(getGithubPullRequestMutationTags).toHaveBeenCalledWith({
      userId: 'user-1',
      owner: 'acme',
      repo: 'widget',
      pullNumber: 42,
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
  })

  it('converts an open pull request back to draft and invalidates pull request tags', async () => {
    convertGithubPullRequestToDraft.mockResolvedValue(undefined)

    const response = await request('/pr/42/convert-to-draft?org=acme&repo=widget', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        pullRequestId: 'PR_kwDOExample',
      }),
    })

    expect(response.status).toBe(204)
    expect(convertGithubPullRequestToDraft).toHaveBeenCalledWith({
      token: 'github-token',
      pullRequestId: 'PR_kwDOExample',
    })
    expect(getGithubPullRequestMutationTags).toHaveBeenCalledWith({
      userId: 'user-1',
      owner: 'acme',
      repo: 'widget',
      pullNumber: 42,
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
  })

  it.each([
    ['/pr/42/ready-for-review?org=acme&repo=widget', markGithubPullRequestReadyForReview],
    ['/pr/42/convert-to-draft?org=acme&repo=widget', convertGithubPullRequestToDraft],
  ])('returns 400 when pullRequestId is missing for %s', async (path) => {
    const response = await request(path, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({}),
    })

    expect(response.status).toBe(400)
    await expect(response.json()).resolves.toEqual({
      error: 'Missing pull request id',
    })
  })

  it.each([
    [403, '/pr/42/ready-for-review?org=acme&repo=widget', markGithubPullRequestReadyForReview],
    [422, '/pr/42/convert-to-draft?org=acme&repo=widget', convertGithubPullRequestToDraft],
  ])(
    'passes through GitHub status %i without invalidating cache for %s',
    async (status, path, mockedCall) => {
      mockedCall.mockRejectedValue(
        Object.assign(new Error(`GitHub draft status error ${status}`), { status }),
      )

      const response = await request(path, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          pullRequestId: 'PR_kwDOExample',
        }),
      })

      expect(response.status).toBe(status)
      expect(invalidateTags).not.toHaveBeenCalled()
      await expect(response.json()).resolves.toEqual({
        error: `GitHub draft status error ${status}`,
      })
    },
  )
})
