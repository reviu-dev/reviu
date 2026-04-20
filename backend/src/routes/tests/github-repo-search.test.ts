import { beforeEach, describe, expect, it, vi } from 'vitest'

const fetchGithubRepositorySearchGraphql = vi.fn()

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
    fetchGithubRepositorySearchGraphql,
  }
})

vi.mock('../../plugins/github/metrics/github-metrics-context.js', () => ({
  runWithGithubMetricsContext: (_context: unknown, callback: () => Promise<unknown>) => callback(),
}))

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github repository search route', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns empty list when query is blank without calling GitHub', async () => {
    const response = await request('/search?q=%20%20')

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({ repositories: [] })
    expect(fetchGithubRepositorySearchGraphql).not.toHaveBeenCalled()
  })

  it('forwards the trimmed query to GitHub and returns mapped repositories', async () => {
    fetchGithubRepositorySearchGraphql.mockResolvedValue({
      repositoryCount: 1,
      repositories: [
        {
          owner: 'acme',
          name: 'reviu',
          full_name: 'acme/reviu',
          description: 'A desktop Git client',
          stars: 42,
          private: false,
          owner_avatar_url: 'https://avatars.githubusercontent.com/u/1?v=4',
        },
      ],
    })

    const response = await request('/search?q=%20reviu%20')

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({
      repositories: [
        {
          owner: 'acme',
          name: 'reviu',
          full_name: 'acme/reviu',
          description: 'A desktop Git client',
          stars: 42,
          private: false,
          owner_avatar_url: 'https://avatars.githubusercontent.com/u/1?v=4',
        },
      ],
    })
    expect(fetchGithubRepositorySearchGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      query: 'reviu',
      limit: 10,
    })
  })

  it('returns 502 when GitHub errors', async () => {
    fetchGithubRepositorySearchGraphql.mockRejectedValue(new Error('boom'))

    const response = await request('/search?q=reviu')

    expect(response.status).toBe(502)
    await expect(response.json()).resolves.toEqual({ error: 'boom' })
  })
})
