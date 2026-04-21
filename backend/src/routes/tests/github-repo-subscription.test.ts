import { beforeEach, describe, expect, it, vi } from 'vitest'

const setGithubRepositorySubscription = vi.fn()

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
    setGithubRepositorySubscription,
  }
})

vi.mock('../../plugins/github/metrics/github-metrics-context.js', () => ({
  runWithGithubMetricsContext: (_context: unknown, callback: () => Promise<unknown>) => callback(),
}))

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github repository subscription routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('pUT rejects an unknown mode', async () => {
    const response = await request('/repos/acme/reviu/subscription', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ mode: 'bogus' }),
    })

    expect(response.status).toBe(400)
    expect(setGithubRepositorySubscription).not.toHaveBeenCalled()
  })

  it('pUT forwards the requested mode to GitHub', async () => {
    setGithubRepositorySubscription.mockResolvedValue({ mode: 'ignore' })

    const response = await request('/repos/acme/reviu/subscription', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ mode: 'ignore' }),
    })

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({ mode: 'ignore' })
    expect(setGithubRepositorySubscription).toHaveBeenCalledWith({
      token: 'github-token',
      owner: 'acme',
      repo: 'reviu',
      mode: 'ignore',
    })
  })

  it('pUT returns 502 when GitHub errors', async () => {
    setGithubRepositorySubscription.mockRejectedValue(new Error('boom'))

    const response = await request('/repos/acme/reviu/subscription', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ mode: 'all' }),
    })

    expect(response.status).toBe(502)
    await expect(response.json()).resolves.toEqual({ error: 'boom' })
  })
})
