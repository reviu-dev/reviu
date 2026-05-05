import { beforeEach, describe, expect, it, vi } from 'vitest'

const getAiSettings = vi.fn()
const saveAiSettings = vi.fn()
const deleteAiSettings = vi.fn()
const generateGithubPrBrief = vi.fn()

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

vi.mock('../../plugins/ai/service.js', () => ({
  getAiSettings,
  saveAiSettings,
  deleteAiSettings,
  generateGithubPrBrief,
}))

async function request(path: string, init?: RequestInit) {
  const { aiRoutes } = await import('../ai.js')
  return aiRoutes.request(`http://localhost${path}`, init)
}

describe('ai routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns redacted AI settings for the current user', async () => {
    getAiSettings.mockResolvedValue({
      configured: true,
      credentialMode: 'user_key',
      provider: 'openai',
      model: 'gpt-5.4-mini',
      apiKeyHint: 'sk-1...abcd',
    })

    const response = await request('/settings')

    expect(response.status).toBe(200)
    expect(getAiSettings).toHaveBeenCalledWith('user-1')
    await expect(response.json()).resolves.toEqual({
      settings: {
        configured: true,
        credentialMode: 'user_key',
        provider: 'openai',
        model: 'gpt-5.4-mini',
        apiKeyHint: 'sk-1...abcd',
      },
    })
  })

  it('saves BYOK AI settings without returning the raw key', async () => {
    saveAiSettings.mockResolvedValue({
      configured: true,
      credentialMode: 'user_key',
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
      apiKeyHint: 'sk-a...wxyz',
    })

    const response = await request('/settings', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        provider: 'anthropic',
        model: 'claude-sonnet-4-6',
        apiKey: 'sk-ant-secret-wxyz',
      }),
    })

    expect(response.status).toBe(200)
    expect(saveAiSettings).toHaveBeenCalledWith('user-1', {
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
      apiKey: 'sk-ant-secret-wxyz',
    })
    await expect(response.json()).resolves.toEqual({
      settings: {
        configured: true,
        credentialMode: 'user_key',
        provider: 'anthropic',
        model: 'claude-sonnet-4-6',
        apiKeyHint: 'sk-a...wxyz',
      },
    })
  })

  it('deletes AI settings for the current user', async () => {
    deleteAiSettings.mockResolvedValue(undefined)

    const response = await request('/settings', { method: 'DELETE' })

    expect(response.status).toBe(200)
    expect(deleteAiSettings).toHaveBeenCalledWith('user-1')
    await expect(response.json()).resolves.toEqual({ ok: true })
  })

  it('generates a PR brief from PR identity instead of client-supplied PR context', async () => {
    generateGithubPrBrief.mockResolvedValue({
      generatedAt: '2026-05-04T12:00:00.000Z',
      owner: 'acme',
      repo: 'widget',
      pullNumber: 42,
      headSha: 'head-sha',
      contextHash: 'context-hash',
      provider: 'openai',
      credentialMode: 'user_key',
      model: 'gpt-5.4-mini',
      cached: false,
      summary: ['This PR updates the billing page.'],
      reviewFirst: [],
      risks: [],
      blockers: [],
    })

    const response = await request('/github/pr/brief', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        owner: 'acme',
        repo: 'widget',
        pullNumber: 42,
        forceRefresh: true,
        ignoredClientContext: {
          title: 'client should not provide PR data',
        },
      }),
    })

    expect(response.status).toBe(200)
    expect(generateGithubPrBrief).toHaveBeenCalledWith({
      userId: 'user-1',
      githubToken: 'github-token',
      owner: 'acme',
      repo: 'widget',
      pullNumber: 42,
      forceRefresh: true,
    })
    await expect(response.json()).resolves.toEqual({
      brief: expect.objectContaining({
        owner: 'acme',
        repo: 'widget',
        pullNumber: 42,
        summary: ['This PR updates the billing page.'],
      }),
    })
  })

  it('returns a conflict when AI settings are not configured', async () => {
    generateGithubPrBrief.mockRejectedValue(
      Object.assign(new Error('AI settings are not configured.'), { status: 409 }),
    )

    const response = await request('/github/pr/brief', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        owner: 'acme',
        repo: 'widget',
        pullNumber: 42,
      }),
    })

    expect(response.status).toBe(409)
    await expect(response.json()).resolves.toEqual({
      error: 'AI settings are not configured.',
    })
  })
})
