import type { GithubCacheGetOrLoadOptions } from '../../plugins/github/cache/github-cache.js'
import type {
  GithubIssueDetails,
  GithubPullRequestConversation,
  GithubReactionGroup,
} from '../../plugins/github/types.js'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const addGithubReactionGraphql = vi.fn()
const fetchGithubIssueDetailsGraphql = vi.fn()
const fetchGithubIssueReferenceTarget = vi.fn()
const fetchGithubPullRequestConversationGraphql = vi.fn()
const removeGithubReactionGraphql = vi.fn()
const resolveGithubPullRequestReviewThreadGraphql = vi.fn()
const unresolveGithubPullRequestReviewThreadGraphql = vi.fn()
const invalidateTags = vi.fn()
const getOrLoad = vi.fn(async <T>(options: GithubCacheGetOrLoadOptions<T>) => {
  const loaded = await options.load({ cachedEntry: null })
  if ('notModified' in loaded) {
    throw new Error('Unexpected not modified payload in test')
  }

  return {
    payload: loaded.payload,
    cacheStatus: 'miss',
    scope: options.scope,
  }
})

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
    addGithubReactionGraphql,
    fetchGithubIssueDetailsGraphql,
    fetchGithubIssueReferenceTarget,
    fetchGithubPullRequestConversationGraphql,
    removeGithubReactionGraphql,
    resolveGithubPullRequestReviewThreadGraphql,
    unresolveGithubPullRequestReviewThreadGraphql,
  }
})

vi.mock('../../plugins/github/cache/github-cache-runtime.js', () => ({
  githubCache: {
    getOrLoad,
    invalidateTags,
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

function makeConversation(): GithubPullRequestConversation {
  return {
    pull_request: {
      node_id: 'PR_kwDOExample',
      reactions: [{
        content: 'THUMBS_UP',
        count: 2,
        viewer_has_reacted: true,
      }],
    },
    issue_comments: [{
      node_id: 'IC_kwDOExample',
      reactions: [{
        content: 'THUMBS_UP',
        count: 2,
        viewer_has_reacted: true,
      }],
      id: 11,
      body: 'Can you add tests?',
      created_at: '2026-02-28T10:00:00Z',
      updated_at: '2026-02-28T10:05:00Z',
      user: { login: 'octocat', avatar_url: '' },
    }],
    reviews: [{
      node_id: 'PRR_kwDOExample',
      reactions: [],
      id: 123,
      body: 'Looks good',
      state: 'APPROVED',
      submitted_at: '2026-02-28T12:00:00Z',
      commit_id: '1111111111111111111111111111111111111111',
      html_url: 'https://github.com/acme/widget/pull/42#pullrequestreview-123',
      user: { login: 'reviewer', avatar_url: '' },
    }],
    review_comments: [{
      node_id: 'PRRC_kwDOExample',
      reactions: [],
      is_outdated: false,
      thread_id: 'PRRT_kwDOExample',
      is_resolved: false,
      is_collapsed: false,
      viewer_can_resolve: true,
      viewer_can_unresolve: false,
      id: 1,
      pull_request_review_id: 123,
      diff_hunk: '@@ -1 +1 @@',
      path: 'src/main.rs',
      position: 1,
      original_position: 1,
      commit_id: 'head123',
      original_commit_id: 'base123',
      in_reply_to_id: undefined,
      user: { login: 'octocat', avatar_url: '' },
      body: 'Looks good',
      created_at: '2026-02-15T12:00:00Z',
      updated_at: '2026-02-15T12:01:00Z',
      start_line: null,
      original_start_line: null,
      start_side: undefined,
      line: 1,
      original_line: 1,
      side: 'RIGHT',
    }],
  }
}

function makeReactions(): GithubReactionGroup[] {
  return [{
    content: 'THUMBS_UP',
    count: 3,
    viewer_has_reacted: true,
  }]
}

function makeIssueDetails(): GithubIssueDetails {
  return {
    node_id: 'I_kwDOExample',
    reactions: [{
      content: 'THUMBS_UP',
      count: 2,
      viewer_has_reacted: true,
    }],
    id: 501,
    number: 77,
    title: 'Fix auth race condition',
    body: 'Issue body',
    state: 'closed',
    state_reason: 'completed',
    created_at: '2026-02-20T08:00:00Z',
    updated_at: '2026-02-21T09:30:00Z',
    closed_at: '2026-02-21T09:30:00Z',
    labels: [{
      name: 'bug',
      color: 'f29513',
    }],
    comments: [{
      node_id: 'IC_kwDOIssue',
      reactions: [],
      id: 9001,
      body: 'Looks good',
      created_at: '2026-02-20T10:00:00Z',
      updated_at: '2026-02-20T10:05:00Z',
      user: { login: 'octocat', avatar_url: '' },
    }],
    user: { login: 'octocat', avatar_url: '' },
    repository: { owner: 'acme', repo: 'widget' },
  }
}

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github pull request conversation route', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    getOrLoad.mockImplementation(async <T>(options: GithubCacheGetOrLoadOptions<T>) => {
      const loaded = await options.load({ cachedEntry: null })
      if ('notModified' in loaded) {
        throw new Error('Unexpected not modified payload in test')
      }

      return {
        payload: loaded.payload,
        cacheStatus: 'miss',
        scope: options.scope,
      }
    })
  })

  it('returns the GraphQL pull request conversation payload', async () => {
    fetchGithubPullRequestConversationGraphql.mockResolvedValue(makeConversation())

    const response = await request('/pr/42/conversation?org=acme&repo=widget')

    expect(response.status).toBe(200)
    expect(fetchGithubPullRequestConversationGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
      pullNumber: 42,
    })
    expect(response.headers.get('x-reviu-cache')).toBe('miss')
    await expect(response.json()).resolves.toEqual({
      conversation: expect.objectContaining({
        issue_comments: [expect.objectContaining({ id: 11 })],
        reviews: [expect.objectContaining({ id: 123 })],
        review_comments: [expect.objectContaining({ id: 1 })],
      }),
    })
  })

  it('validates required query params', async () => {
    const response = await request('/pr/42/conversation?org=acme')

    expect(response.status).toBe(400)
    await expect(response.json()).resolves.toEqual({
      error: 'Missing org, repo, or id',
    })
    expect(fetchGithubPullRequestConversationGraphql).not.toHaveBeenCalled()
  })

  it.each([403, 404, 422])('passes through GitHub status %i', async (status) => {
    fetchGithubPullRequestConversationGraphql.mockRejectedValue(
      Object.assign(new Error(`GitHub conversation error ${status}`), { status }),
    )

    const response = await request('/pr/42/conversation?org=acme&repo=widget')

    expect(response.status).toBe(status)
    await expect(response.json()).resolves.toEqual({
      error: `GitHub conversation error ${status}`,
    })
  })

  it('adds a pull request conversation reaction', async () => {
    addGithubReactionGraphql.mockResolvedValue(makeReactions())

    const response = await request('/pr/42/reactions?org=acme&repo=widget', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        subjectId: 'IC_kwDOExample',
        content: 'THUMBS_UP',
      }),
    })

    expect(response.status).toBe(200)
    expect(addGithubReactionGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      subjectId: 'IC_kwDOExample',
      content: 'THUMBS_UP',
    })
    expect(invalidateTags).toHaveBeenCalledWith(expect.arrayContaining([
      'issue:acme/widget:42:comments',
      'pull-request:acme/widget:42:comments',
      'pull-request:acme/widget:42:reviews',
    ]))
    await expect(response.json()).resolves.toEqual({
      reactions: makeReactions(),
    })
  })

  it('resolves a review thread and invalidates conversation cache tags', async () => {
    resolveGithubPullRequestReviewThreadGraphql.mockResolvedValue({
      thread_id: 'PRRT_kwDOExample',
      is_resolved: true,
      viewer_can_resolve: false,
      viewer_can_unresolve: true,
    })

    const response = await request(
      '/pr/42/review-threads/PRRT_kwDOExample/resolve?org=acme&repo=widget',
      { method: 'POST' },
    )

    expect(response.status).toBe(200)
    expect(resolveGithubPullRequestReviewThreadGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      threadId: 'PRRT_kwDOExample',
    })
    expect(invalidateTags).toHaveBeenCalledWith(expect.arrayContaining([
      'pull-request:acme/widget:42:comments',
    ]))
    await expect(response.json()).resolves.toEqual({
      thread: {
        thread_id: 'PRRT_kwDOExample',
        is_resolved: true,
        viewer_can_resolve: false,
        viewer_can_unresolve: true,
      },
    })
  })

  it('unresolves a review thread', async () => {
    unresolveGithubPullRequestReviewThreadGraphql.mockResolvedValue({
      thread_id: 'PRRT_kwDOExample',
      is_resolved: false,
      viewer_can_resolve: true,
      viewer_can_unresolve: false,
    })

    const response = await request(
      '/pr/42/review-threads/PRRT_kwDOExample/unresolve?org=acme&repo=widget',
      { method: 'POST' },
    )

    expect(response.status).toBe(200)
    expect(unresolveGithubPullRequestReviewThreadGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      threadId: 'PRRT_kwDOExample',
    })
  })

  it('passes through forbidden errors when resolving a review thread', async () => {
    resolveGithubPullRequestReviewThreadGraphql.mockRejectedValue(
      Object.assign(new Error('Not allowed'), { status: 403 }),
    )

    const response = await request(
      '/pr/42/review-threads/PRRT_kwDOExample/resolve?org=acme&repo=widget',
      { method: 'POST' },
    )

    expect(response.status).toBe(403)
    await expect(response.json()).resolves.toEqual({ error: 'Not allowed' })
    expect(invalidateTags).not.toHaveBeenCalled()
  })

  it('removes a pull request conversation reaction', async () => {
    removeGithubReactionGraphql.mockResolvedValue([])

    const response = await request('/pr/42/reactions?org=acme&repo=widget', {
      method: 'DELETE',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        subjectId: 'PRRC_kwDOExample',
        content: 'HEART',
      }),
    })

    expect(response.status).toBe(200)
    expect(removeGithubReactionGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      subjectId: 'PRRC_kwDOExample',
      content: 'HEART',
    })
    await expect(response.json()).resolves.toEqual({
      reactions: [],
    })
  })

  it('returns the GraphQL issue details payload', async () => {
    fetchGithubIssueDetailsGraphql.mockResolvedValue(makeIssueDetails())

    const response = await request('/repos/acme/widget/issues/77')

    expect(response.status).toBe(200)
    expect(fetchGithubIssueDetailsGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
      issueNumber: 77,
    })
    expect(response.headers.get('x-reviu-cache')).toBe('miss')
    await expect(response.json()).resolves.toEqual({
      issue: expect.objectContaining({
        node_id: 'I_kwDOExample',
        reactions: [expect.objectContaining({ content: 'THUMBS_UP' })],
        comments: [expect.objectContaining({ node_id: 'IC_kwDOIssue' })],
      }),
    })
  })

  it('resolves issue-number links to pull request targets', async () => {
    fetchGithubIssueReferenceTarget.mockResolvedValue({
      kind: 'pull_request',
      number: 24877,
    })

    const response = await request('/repos/acme/widget/issues/24877/target')

    expect(response.status).toBe(200)
    expect(fetchGithubIssueReferenceTarget).toHaveBeenCalledWith({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
      issueNumber: 24877,
    })
    await expect(response.json()).resolves.toEqual({
      target: {
        kind: 'pull_request',
        number: 24877,
      },
    })
  })

  it('adds an issue reaction and invalidates issue details cache tags', async () => {
    addGithubReactionGraphql.mockResolvedValue(makeReactions())

    const response = await request('/repos/acme/widget/issues/77/reactions', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        subjectId: 'IC_kwDOIssue',
        content: 'THUMBS_UP',
      }),
    })

    expect(response.status).toBe(200)
    expect(addGithubReactionGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      subjectId: 'IC_kwDOIssue',
      content: 'THUMBS_UP',
    })
    expect(invalidateTags).toHaveBeenCalledWith(expect.arrayContaining([
      'issue:acme/widget:77',
      'issue:acme/widget:77:comments',
    ]))
    await expect(response.json()).resolves.toEqual({
      reactions: makeReactions(),
    })
  })

  it('passes through issue reaction OAuth app access restriction errors', async () => {
    addGithubReactionGraphql.mockRejectedValue(
      Object.assign(
        new Error('The openai organization restricts OAuth app access. Ask an organization owner to approve Reviu, then try again.'),
        { status: 403 },
      ),
    )

    const response = await request('/repos/openai/reviu/issues/77/reactions', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        subjectId: 'I_kwDOExample',
        content: 'THUMBS_UP',
      }),
    })

    expect(response.status).toBe(403)
    await expect(response.json()).resolves.toEqual({
      error: 'The openai organization restricts OAuth app access. Ask an organization owner to approve Reviu, then try again.',
    })
  })

  it('removes an issue reaction', async () => {
    removeGithubReactionGraphql.mockResolvedValue([])

    const response = await request('/repos/acme/widget/issues/77/reactions', {
      method: 'DELETE',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        subjectId: 'I_kwDOExample',
        content: 'HEART',
      }),
    })

    expect(response.status).toBe(200)
    expect(removeGithubReactionGraphql).toHaveBeenCalledWith({
      token: 'github-token',
      subjectId: 'I_kwDOExample',
      content: 'HEART',
    })
    await expect(response.json()).resolves.toEqual({
      reactions: [],
    })
  })
})
