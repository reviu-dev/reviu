import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetchGithubUserProfileGraphql = vi.fn()

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
    fetchGithubUserProfileGraphql,
  }
})

vi.mock('../../plugins/github/metrics/github-metrics-context.js', () => ({
  runWithGithubMetricsContext: (_context: unknown, callback: () => Promise<unknown>) => callback(),
}))

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github user profile route', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns a mapped profile for a GitHub login', async () => {
    fetchGithubUserProfileGraphql.mockResolvedValue({
      login: 'octocat',
      name: 'The Octocat',
      avatar_url: 'https://avatars.githubusercontent.com/u/583231?v=4',
      bio: 'GitHub mascot',
      company: '@github',
      location: 'San Francisco',
      website_url: 'https://github.blog',
      twitter_username: 'octocat',
      html_url: 'https://github.com/octocat',
      created_at: '2011-01-25T18:44:36Z',
      followers_count: 99,
      following_count: 5,
      repositories_count: 2,
      repositories_indexed_count: 2,
      repositories_truncated: false,
      stargazers_count: 20,
      forks_count: 4,
      languages: [],
      repositories: [],
    })

    const response = await request('/users/octocat')

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toMatchObject({
      login: 'octocat',
      followers_count: 99,
      stargazers_count: 20,
    })
    expect(fetchGithubUserProfileGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      login: 'octocat',
      repositoriesLimit: 100,
    })
  })

  it('returns 404 when the GitHub user is missing', async () => {
    fetchGithubUserProfileGraphql.mockResolvedValue(null)

    const response = await request('/users/missing')

    expect(response.status).toBe(404)
    await expect(response.json()).resolves.toEqual({ error: 'GitHub user not found' })
  })

  it('returns 502 when GitHub errors', async () => {
    fetchGithubUserProfileGraphql.mockRejectedValue(new Error('boom'))

    const response = await request('/users/octocat')

    expect(response.status).toBe(502)
    await expect(response.json()).resolves.toEqual({ error: 'boom' })
  })
})
