import { beforeEach, describe, expect, it, vi } from 'vitest'

const createFeedbackIssue = vi.fn()

vi.mock('../middlewares/auth.js', async () => {
  const { createMiddleware } = await import('hono/factory')

  return {
    authMiddlewareUser: createMiddleware(async (ctx, next) => {
      ctx.set('user', {
        id: 'user-1',
        createdAt: new Date('2026-03-19T00:00:00Z'),
        updatedAt: new Date('2026-03-19T00:00:00Z'),
        email: 'user@example.com',
        emailVerified: true,
        name: 'Reviu Test User',
        image: null,
        proGrantedAt: null,
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

vi.mock('../plugins/feedback/service.js', () => ({
  createFeedbackIssue,
}))

async function request(path: string, init?: RequestInit) {
  const { feedbackRoutes } = await import('./feedback.js')
  return feedbackRoutes.request(`http://localhost${path}`, init)
}

function postFeedback(body: Record<string, unknown>) {
  return request('/', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
}

describe('feedback routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('creates a bug feedback issue and returns 201', async () => {
    createFeedbackIssue.mockResolvedValue({
      issueId: 'issue-1',
      url: 'https://linear.app/team/issue-1',
    })

    const response = await postFeedback({
      type: 'bug',
      title: 'App crashes on startup',
      description: 'The app crashes when I open it',
    })

    expect(response.status).toBe(201)
    const json = await response.json()
    expect(json).toEqual({
      issueId: 'issue-1',
      url: 'https://linear.app/team/issue-1',
    })
    expect(createFeedbackIssue).toHaveBeenCalledWith({
      type: 'bug',
      title: 'App crashes on startup',
      description: 'The app crashes when I open it',
      userEmail: 'user@example.com',
    })
  })

  it('creates a feature feedback issue and returns 201', async () => {
    createFeedbackIssue.mockResolvedValue({
      issueId: 'issue-2',
      url: 'https://linear.app/team/issue-2',
    })

    const response = await postFeedback({
      type: 'feature',
      title: 'Add dark mode',
      description: 'Would love a dark mode option',
    })

    expect(response.status).toBe(201)
    expect(createFeedbackIssue).toHaveBeenCalledWith({
      type: 'feature',
      title: 'Add dark mode',
      description: 'Would love a dark mode option',
      userEmail: 'user@example.com',
    })
  })

  it('returns 400 when title is missing', async () => {
    const response = await postFeedback({
      type: 'bug',
      description: 'Some description',
    })

    expect(response.status).toBe(400)
    expect(createFeedbackIssue).not.toHaveBeenCalled()
  })

  it('returns 400 when type is invalid', async () => {
    const response = await postFeedback({
      type: 'question',
      title: 'A question',
      description: 'Some description',
    })

    expect(response.status).toBe(400)
    expect(createFeedbackIssue).not.toHaveBeenCalled()
  })

  it('returns 400 when title exceeds 200 characters', async () => {
    const response = await postFeedback({
      type: 'bug',
      title: 'a'.repeat(201),
      description: 'Some description',
    })

    expect(response.status).toBe(400)
    expect(createFeedbackIssue).not.toHaveBeenCalled()
  })

  it('returns 502 when service throws', async () => {
    createFeedbackIssue.mockRejectedValue(new Error('Linear API error'))

    const response = await postFeedback({
      type: 'bug',
      title: 'Something broke',
      description: 'Details here',
    })

    expect(response.status).toBe(502)
    const json = await response.json()
    expect(json).toEqual({ error: 'Failed to submit feedback' })
  })

  it('accepts empty description', async () => {
    createFeedbackIssue.mockResolvedValue({
      issueId: 'issue-3',
      url: 'https://linear.app/team/issue-3',
    })

    const response = await postFeedback({
      type: 'bug',
      title: 'Quick bug',
      description: '',
    })

    expect(response.status).toBe(201)
    expect(createFeedbackIssue).toHaveBeenCalledWith({
      type: 'bug',
      title: 'Quick bug',
      description: '',
      userEmail: 'user@example.com',
    })
  })
})
