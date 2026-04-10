import { beforeEach, describe, expect, it, vi } from 'vitest'

const addGithubIssueAssignees = vi.fn()
const removeGithubIssueAssignees = vi.fn()
const addGithubIssueLabels = vi.fn()
const removeGithubIssueLabel = vi.fn()
const requestGithubPullRequestReviewers = vi.fn()
const removeGithubPullRequestReviewers = vi.fn()
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

vi.mock('../../plugins/github/service.js', async () => {
  const actual = await vi.importActual<typeof import('../../plugins/github/service.js')>(
    '../../plugins/github/service.js',
  )

  return {
    ...actual,
    addGithubIssueAssignees,
    addGithubIssueLabels,
    removeGithubIssueAssignees,
    removeGithubIssueLabel,
    requestGithubPullRequestReviewers,
    removeGithubPullRequestReviewers,
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

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github pull request people routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    invalidateTags.mockResolvedValue(undefined)
    getGithubPullRequestMutationTags.mockReturnValue(['pr-tag', 'repo-tag'])
  })

  it('adds an assignee and invalidates pull request tags', async () => {
    addGithubIssueAssignees.mockResolvedValue(undefined)

    const response = await request('/pr/42/assignees?org=acme&repo=widget', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        users: ['alice'],
      }),
    })

    expect(response.status).toBe(204)
    expect(addGithubIssueAssignees).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        issue_number: 42,
        assignees: ['alice'],
      },
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
  })

  it('removes an assignee and invalidates pull request tags', async () => {
    removeGithubIssueAssignees.mockResolvedValue(undefined)

    const response = await request('/pr/42/assignees?org=acme&repo=widget', {
      method: 'DELETE',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        users: ['alice'],
      }),
    })

    expect(response.status).toBe(204)
    expect(removeGithubIssueAssignees).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        issue_number: 42,
        assignees: ['alice'],
      },
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
  })

  it('requests a reviewer and invalidates pull request tags', async () => {
    requestGithubPullRequestReviewers.mockResolvedValue(undefined)

    const response = await request('/pr/42/requested-reviewers?org=acme&repo=widget', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        users: ['bob'],
      }),
    })

    expect(response.status).toBe(204)
    expect(requestGithubPullRequestReviewers).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        pull_number: 42,
        reviewers: ['bob'],
      },
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
  })

  it('adds labels and invalidates pull request tags', async () => {
    addGithubIssueLabels.mockResolvedValue(undefined)

    const response = await request('/pr/42/labels?org=acme&repo=widget', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        labels: ['bug', 'docs'],
      }),
    })

    expect(response.status).toBe(204)
    expect(addGithubIssueLabels).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        issue_number: 42,
        labels: ['bug', 'docs'],
      },
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
  })

  it('removes labels and invalidates pull request tags', async () => {
    removeGithubIssueLabel.mockResolvedValue(undefined)

    const response = await request('/pr/42/labels?org=acme&repo=widget', {
      method: 'DELETE',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        labels: ['bug', 'docs'],
      }),
    })

    expect(response.status).toBe(204)
    expect(removeGithubIssueLabel).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        issue_number: 42,
        name: 'bug',
      },
    })
    expect(removeGithubIssueLabel).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        issue_number: 42,
        name: 'docs',
      },
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
  })

  it('removes a requested reviewer and invalidates pull request tags', async () => {
    removeGithubPullRequestReviewers.mockResolvedValue(undefined)

    const response = await request('/pr/42/requested-reviewers?org=acme&repo=widget', {
      method: 'DELETE',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        users: ['bob'],
      }),
    })

    expect(response.status).toBe(204)
    expect(removeGithubPullRequestReviewers).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        pull_number: 42,
        reviewers: ['bob'],
      },
    })
    expect(invalidateTags).toHaveBeenCalledWith(['pr-tag', 'repo-tag'])
  })

  it.each([
    ['POST', '/pr/42/assignees?org=acme&repo=widget'],
    ['DELETE', '/pr/42/assignees?org=acme&repo=widget'],
    ['POST', '/pr/42/labels?org=acme&repo=widget'],
    ['DELETE', '/pr/42/labels?org=acme&repo=widget'],
    ['POST', '/pr/42/requested-reviewers?org=acme&repo=widget'],
    ['DELETE', '/pr/42/requested-reviewers?org=acme&repo=widget'],
  ])('returns 400 when users are missing for %s %s', async (method, path) => {
    const response = await request(path, {
      method,
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({}),
    })

    expect(response.status).toBe(400)
    await expect(response.json()).resolves.toMatchObject({
      success: false,
      error: {
        name: 'ZodError',
      },
    })
  })

  it.each([
    [422, '/pr/42/assignees?org=acme&repo=widget', 'POST', addGithubIssueAssignees],
    [422, '/pr/42/labels?org=acme&repo=widget', 'POST', addGithubIssueLabels],
    [403, '/pr/42/requested-reviewers?org=acme&repo=widget', 'DELETE', removeGithubPullRequestReviewers],
  ])(
    'passes through GitHub status %i without invalidating cache for %s %s',
    async (status, path, method, mockedCall) => {
      mockedCall.mockRejectedValue(
        Object.assign(new Error(`GitHub people mutation error ${status}`), { status }),
      )

      const response = await request(path, {
        method,
        headers: {
          'content-type': 'application/json',
        },
        body: JSON.stringify(path.includes('/labels')
          ? { labels: ['bug'] }
          : { users: ['octocat'] }),
      })

      expect(response.status).toBe(status)
      expect(invalidateTags).not.toHaveBeenCalled()
      await expect(response.json()).resolves.toEqual({
        error: `GitHub people mutation error ${status}`,
      })
    },
  )
})
