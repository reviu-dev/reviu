import type { ForkRepositoryResponse } from '../../plugins/github/types.js'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const forkGithubRepository = vi.fn()

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
    forkGithubRepository,
  }
})

vi.mock('../../plugins/github/cache/github-cache-runtime.js', () => ({
  githubCache: {
    getOrLoad: vi.fn(),
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

function makeForkResponse(overrides: Partial<ForkRepositoryResponse> = {}): ForkRepositoryResponse {
  return {
    id: 1,
    node_id: 'repo-fork-1',
    name: 'source-repo',
    full_name: 'octocat/source-repo',
    private: false,
    owner: {
      login: 'octocat',
      id: 1,
      node_id: 'user-1',
      avatar_url: 'https://example.com/avatar.png',
      gravatar_id: '',
      url: 'https://api.github.com/users/octocat',
      html_url: 'https://github.com/octocat',
      type: 'User',
      site_admin: false,
    },
    html_url: 'https://github.com/octocat/source-repo',
    description: 'Forked',
    fork: true,
    default_branch: 'main',
    ...overrides,
  } as unknown as ForkRepositoryResponse
}

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github fork repository route', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('forks the repository under the viewer by default', async () => {
    forkGithubRepository.mockResolvedValue(makeForkResponse())

    const response = await request('/repos/acme/source-repo/forks', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ defaultBranchOnly: true }),
    })

    expect(response.status).toBe(202)
    expect(forkGithubRepository).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'source-repo',
        default_branch_only: true,
      },
    })
    await expect(response.json()).resolves.toEqual({
      repository: {
        owner: 'octocat',
        repo: 'source-repo',
        full_name: 'octocat/source-repo',
        description: 'Forked',
        private: false,
        html_url: 'https://github.com/octocat/source-repo',
      },
    })
  })

  it('forks into an organization with a custom name', async () => {
    forkGithubRepository.mockResolvedValue(makeForkResponse({
      name: 'new-name',
      full_name: 'globex/new-name',
      html_url: 'https://github.com/globex/new-name',
      owner: {
        login: 'globex',
        id: 99,
        node_id: 'org-99',
        avatar_url: 'https://example.com/globex.png',
        gravatar_id: '',
        url: 'https://api.github.com/orgs/globex',
        html_url: 'https://github.com/globex',
        type: 'Organization',
        site_admin: false,
      },
    } as unknown as Partial<ForkRepositoryResponse>))

    const response = await request('/repos/acme/source-repo/forks', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        organization: 'globex',
        name: 'new-name',
        defaultBranchOnly: false,
      }),
    })

    expect(response.status).toBe(202)
    expect(forkGithubRepository).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'source-repo',
        organization: 'globex',
        name: 'new-name',
        default_branch_only: false,
      },
    })
    await expect(response.json()).resolves.toMatchObject({
      repository: {
        owner: 'globex',
        repo: 'new-name',
      },
    })
  })

  it('returns 400 when the custom name is invalid', async () => {
    const response = await request('/repos/acme/source-repo/forks', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: 'bad name!' }),
    })

    expect(response.status).toBe(400)
    expect(forkGithubRepository).not.toHaveBeenCalled()
  })

  it('forwards GitHub 404 when the source repository does not exist', async () => {
    const error = Object.assign(new Error('Not Found'), { status: 404 })
    forkGithubRepository.mockRejectedValue(error)

    const response = await request('/repos/acme/missing/forks', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({}),
    })

    expect(response.status).toBe(404)
    await expect(response.json()).resolves.toEqual({ error: 'Not Found' })
  })

  it('forwards GitHub 422 when the fork already exists', async () => {
    const error = Object.assign(new Error('name already exists on this account'), { status: 422 })
    forkGithubRepository.mockRejectedValue(error)

    const response = await request('/repos/acme/source-repo/forks', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({}),
    })

    expect(response.status).toBe(422)
  })
})
