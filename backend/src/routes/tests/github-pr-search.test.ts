import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetchGithubPullRequestSearchGraphql = vi.fn()
const fetchGithubRepositoryLabels = vi.fn()
const fetchGithubRepositoryAssignees = vi.fn()
const getOrLoad = vi.fn()

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
    fetchGithubPullRequestSearchGraphql,
    fetchGithubRepositoryLabels,
    fetchGithubRepositoryAssignees,
  }
})

vi.mock('../../plugins/github/cache/github-cache-runtime.js', () => ({
  githubCache: {
    getOrLoad,
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

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github pull request search routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    getOrLoad.mockImplementation(async ({ load, scope = 'viewer' }: any) => {
      const loaded = await load()
      return {
        ...loaded,
        cacheStatus: 'miss',
        scope,
      }
    })
    fetchGithubPullRequestSearchGraphql.mockResolvedValue({ nodes: [], issueCount: 0 })
    fetchGithubRepositoryLabels.mockResolvedValue([])
    fetchGithubRepositoryAssignees.mockResolvedValue([])
  })

  it('searches pull requests from structured filters via the new POST route', async () => {
    const response = await request('/pr/search', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        filters: {
          repos: ['acme/reviu', 'acme/api'],
          labels: ['bug', 'needs design'],
          authors: ['@me'],
          assignees: ['alice'],
          requested_reviewers: ['@me'],
          review_status: 'required',
          include_drafts: false,
        },
      }),
    })

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({
      pullRequests: [],
    })
    expect(response.headers.get('x-reviu-cache')).toBe('miss')
    expect(response.headers.get('x-reviu-cache-scope')).toBe('viewer')
    expect(fetchGithubPullRequestSearchGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      query: 'is:pr state:open archived:false repo:acme/reviu repo:acme/api (label:"bug" OR label:"needs design") author:@me assignee:alice user-review-requested:@me review:required draft:false sort:updated-desc',
      limit: 20,
    })
  })

  it('uses repeated repo qualifiers for multi-repository pull request searches', async () => {
    const response = await request('/pr/search', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        filters: {
          repos: ['acme/reviu', 'acme/api'],
          labels: [],
          authors: [],
          assignees: [],
          requested_reviewers: [],
          review_status: 'any',
          include_drafts: true,
        },
      }),
    })

    expect(response.status).toBe(200)
    expect(fetchGithubPullRequestSearchGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      query: 'is:pr state:open archived:false repo:acme/reviu repo:acme/api sort:updated-desc',
      limit: 20,
    })
  })

  it('searches repository pull requests through the existing repo list route', async () => {
    const response = await request(
      '/repos/acme/widget/pr?state=closed&label=bug&label=needs%20design&author=%40me&assignee=alice&requested_reviewer=bob&review_status=changes_requested&include_drafts=false&base=main&sort=comments_desc',
    )

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({
      pullRequests: [],
      pullRequestCount: 0,
      page: 1,
      perPage: 30,
      totalPages: 1,
    })
    expect(fetchGithubPullRequestSearchGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      query: 'is:pr archived:false repo:acme/widget (label:"bug" OR label:"needs design") author:@me assignee:alice review-requested:bob review:changes_requested draft:false base:main sort:comments-desc state:closed -is:merged',
      limit: 30,
    })
  })

  it('uses GitHub Search for repository pull requests without filters', async () => {
    const response = await request('/repos/acme/widget/pr?state=open')

    expect(response.status).toBe(200)
    expect(fetchGithubPullRequestSearchGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      query: 'is:pr archived:false repo:acme/widget sort:updated-desc state:open',
      limit: 30,
    })
  })

  it('rejects missing state parameter for repository pull requests', async () => {
    const response = await request('/repos/acme/widget/pr')

    expect(response.status).toBe(400)
    expect(fetchGithubPullRequestSearchGraphql).not.toHaveBeenCalled()
  })

  it('rejects invalid repository pull request filter values', async () => {
    const response = await request('/repos/acme/widget/pr?state=open&sort=unknown')

    expect(response.status).toBe(400)
    expect(fetchGithubPullRequestSearchGraphql).not.toHaveBeenCalled()
  })

  it('aggregates label, assignee, and author filter options for selected repositories', async () => {
    fetchGithubRepositoryLabels.mockImplementation(async ({ params }: any) => {
      if (params.repo === 'reviu') {
        return [{ name: 'bug' }, { name: 'enhancement' }]
      }

      return [{ name: 'bug' }, { name: 'docs' }]
    })
    fetchGithubRepositoryAssignees.mockImplementation(async ({ params }: any) => {
      if (params.repo === 'reviu') {
        return [
          { login: 'alice', avatar_url: 'https://example.com/alice.png' },
          { login: 'bob', avatar_url: null },
        ]
      }

      return [
        { login: 'alice', avatar_url: 'https://example.com/alice.png' },
        { login: 'carol', avatar_url: 'https://example.com/carol.png' },
      ]
    })
    fetchGithubPullRequestSearchGraphql.mockResolvedValue({
      nodes: [
        {
          author: {
            login: 'octocat',
            avatarUrl: 'https://example.com/octocat.png',
          },
        },
        {
          author: {
            login: 'renovate[bot]',
            avatarUrl: null,
          },
        },
        {
          author: {
            login: 'octocat',
            avatarUrl: 'https://example.com/octocat.png',
          },
        },
      ],
      issueCount: 3,
    })

    const response = await request('/pr/filter-options', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        repos: [' acme/reviu ', 'acme/api', 'acme/reviu'],
      }),
    })

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({
      options: {
        labels: [
          { name: 'bug' },
          { name: 'docs' },
          { name: 'enhancement' },
        ],
        authors: [
          { login: 'octocat', avatar_url: 'https://example.com/octocat.png' },
          { login: 'renovate[bot]', avatar_url: null },
        ],
        assignees: [
          { login: 'alice', avatar_url: 'https://example.com/alice.png' },
          { login: 'bob', avatar_url: null },
          { login: 'carol', avatar_url: 'https://example.com/carol.png' },
        ],
      },
    })
    expect(fetchGithubRepositoryLabels).toHaveBeenCalledTimes(2)
    expect(fetchGithubRepositoryAssignees).toHaveBeenCalledTimes(2)
    expect(fetchGithubPullRequestSearchGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      query: 'is:pr state:open archived:false repo:acme/reviu repo:acme/api sort:updated-desc',
      limit: 50,
    })
  })
})
