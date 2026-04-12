import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetchGithubPullRequest = vi.fn()
const updateGithubPullRequestBranch = vi.fn()
const invalidateTags = vi.fn()
const getGithubPullRequestMutationTags = vi.fn()
const getGithubPullRequestCommitsTag = vi.fn()
const getGithubPullRequestFilesTag = vi.fn()

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

vi.mock('../../plugins/github/service.js', async () => {
  const actual = await vi.importActual<typeof import('../../plugins/github/service.js')>(
    '../../plugins/github/service.js',
  )

  return {
    ...actual,
    fetchGithubPullRequest,
    updateGithubPullRequestBranch,
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
    getGithubPullRequestCommitsTag,
    getGithubPullRequestFilesTag,
  }
})

vi.mock('../../plugins/github/metrics/github-metrics-context.js', () => ({
  runWithGithubMetricsContext: (_context: unknown, callback: () => Promise<unknown>) => callback(),
}))

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github pull request update branch route', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    invalidateTags.mockResolvedValue(undefined)
    updateGithubPullRequestBranch.mockResolvedValue(undefined)
    getGithubPullRequestMutationTags.mockReturnValue(['pr-tag', 'repo-tag'])
    getGithubPullRequestCommitsTag.mockReturnValue('commits-tag')
    getGithubPullRequestFilesTag.mockReturnValue('files-tag')
  })

  it('waits for the pull request head to change before invalidating commits and files cache tags', async () => {
    fetchGithubPullRequest
      .mockResolvedValueOnce({ head: { sha: 'head-before' } })
      .mockResolvedValueOnce({ head: { sha: 'head-after' } })

    const response = await request('/pr/42/update-branch?org=acme&repo=widget', {
      method: 'PUT',
    })

    expect(response.status).toBe(202)
    expect(fetchGithubPullRequest).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        pull_number: 42,
      },
    })
    expect(updateGithubPullRequestBranch).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        pull_number: 42,
        expected_head_sha: 'head-before',
      },
    })
    expect(getGithubPullRequestMutationTags).toHaveBeenCalledWith({
      userId: 'user-1',
      owner: 'acme',
      repo: 'widget',
      pullNumber: 42,
    })
    expect(getGithubPullRequestCommitsTag).toHaveBeenCalledWith('acme', 'widget', 42)
    expect(getGithubPullRequestFilesTag).toHaveBeenCalledWith('acme', 'widget', 42)
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag', 'commits-tag', 'files-tag'])
    expect(fetchGithubPullRequest).toHaveBeenCalledTimes(2)
    expect(updateGithubPullRequestBranch.mock.invocationCallOrder[0]).toBeLessThan(
      fetchGithubPullRequest.mock.invocationCallOrder[1],
    )
    expect(fetchGithubPullRequest.mock.invocationCallOrder[1]).toBeLessThan(
      invalidateTags.mock.invocationCallOrder[0],
    )
  })

  it.each([403, 404, 422])(
    'passes through update branch status %i from GitHub without invalidating cache',
    async (status) => {
      fetchGithubPullRequest.mockResolvedValueOnce({ head: { sha: 'head-before' } })
      updateGithubPullRequestBranch.mockRejectedValue(
        Object.assign(new Error(`GitHub update branch error ${status}`), { status }),
      )

      const response = await request('/pr/42/update-branch?org=acme&repo=widget', {
        method: 'PUT',
      })

      expect(response.status).toBe(status)
      expect(invalidateTags).not.toHaveBeenCalled()
      await expect(response.json()).resolves.toEqual({
        error: `GitHub update branch error ${status}`,
      })
    },
  )
})
