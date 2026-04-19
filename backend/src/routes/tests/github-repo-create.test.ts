import type {
  CreateOrgRepositoryResponse,
  CreateUserRepositoryResponse,
  UserOrganizationResponse,
} from '../../plugins/github/types.js'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const createGithubRepositoryForUser = vi.fn()
const createGithubRepositoryForOrg = vi.fn()
const fetchGithubUserOrganizations = vi.fn()

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
    createGithubRepositoryForUser,
    createGithubRepositoryForOrg,
    fetchGithubUserOrganizations,
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

function makeRepository(overrides: Partial<CreateUserRepositoryResponse> = {}): CreateUserRepositoryResponse {
  return {
    id: 1,
    node_id: 'repo-1',
    name: 'my-repo',
    full_name: 'octocat/my-repo',
    private: true,
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
    html_url: 'https://github.com/octocat/my-repo',
    description: 'A brand new repository',
    fork: false,
    default_branch: 'main',
    ...overrides,
  } as unknown as CreateUserRepositoryResponse
}

function makeOrganization(overrides: Partial<UserOrganizationResponse> = {}): UserOrganizationResponse {
  return {
    login: 'acme',
    id: 42,
    node_id: 'org-42',
    url: 'https://api.github.com/orgs/acme',
    repos_url: '',
    events_url: '',
    hooks_url: '',
    issues_url: '',
    members_url: '',
    public_members_url: '',
    avatar_url: 'https://example.com/acme.png',
    description: 'Acme organization',
    ...overrides,
  } as unknown as UserOrganizationResponse
}

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('../github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github user organizations route', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns the viewer organizations', async () => {
    fetchGithubUserOrganizations.mockResolvedValue([
      makeOrganization({ login: 'acme', avatar_url: 'https://example.com/acme.png' }),
      makeOrganization({ login: 'globex', avatar_url: 'https://example.com/globex.png' }),
    ])

    const response = await request('/user/orgs')

    expect(response.status).toBe(200)
    expect(fetchGithubUserOrganizations).toHaveBeenCalledWith({ token: 'github-token' })
    await expect(response.json()).resolves.toEqual({
      organizations: [
        { login: 'acme', avatarUrl: 'https://example.com/acme.png', description: 'Acme organization' },
        { login: 'globex', avatarUrl: 'https://example.com/globex.png', description: 'Acme organization' },
      ],
    })
  })
})

describe('github create repository route (viewer)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('creates a repository for the authenticated user', async () => {
    createGithubRepositoryForUser.mockResolvedValue(makeRepository())

    const response = await request('/repos', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        name: 'my-repo',
        description: 'A brand new repository',
        private: true,
      }),
    })

    expect(response.status).toBe(201)
    expect(createGithubRepositoryForUser).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        name: 'my-repo',
        description: 'A brand new repository',
        private: true,
        auto_init: true,
      },
    })
    await expect(response.json()).resolves.toEqual({
      repository: {
        owner: 'octocat',
        repo: 'my-repo',
        full_name: 'octocat/my-repo',
        description: 'A brand new repository',
        private: true,
        html_url: 'https://github.com/octocat/my-repo',
      },
    })
  })

  it('returns 400 when the name is missing', async () => {
    const response = await request('/repos', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: '   ', private: true }),
    })

    expect(response.status).toBe(400)
    await expect(response.json()).resolves.toEqual({
      error: expect.stringContaining('Missing repository name'),
    })
    expect(createGithubRepositoryForUser).not.toHaveBeenCalled()
  })

  it('returns 400 when the name contains invalid characters', async () => {
    const response = await request('/repos', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: 'bad name!', private: true }),
    })

    expect(response.status).toBe(400)
    expect(createGithubRepositoryForUser).not.toHaveBeenCalled()
  })

  it('forwards GitHub 422 errors to the client', async () => {
    const error = Object.assign(new Error('name already exists on this account'), { status: 422 })
    createGithubRepositoryForUser.mockRejectedValue(error)

    const response = await request('/repos', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: 'my-repo', private: true }),
    })

    expect(response.status).toBe(422)
    await expect(response.json()).resolves.toEqual({
      error: 'name already exists on this account',
    })
  })
})

describe('github create repository route (organization)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('creates a repository for the given organization', async () => {
    createGithubRepositoryForOrg.mockResolvedValue(makeRepository({
      name: 'org-repo',
      full_name: 'acme/org-repo',
      html_url: 'https://github.com/acme/org-repo',
      owner: {
        login: 'acme',
        id: 42,
        node_id: 'org-42',
        avatar_url: 'https://example.com/acme.png',
        gravatar_id: '',
        url: 'https://api.github.com/orgs/acme',
        html_url: 'https://github.com/acme',
        type: 'Organization',
        site_admin: false,
      },
    } as unknown as Partial<CreateOrgRepositoryResponse>))

    const response = await request('/orgs/acme/repos', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        name: 'org-repo',
        private: false,
      }),
    })

    expect(response.status).toBe(201)
    expect(createGithubRepositoryForOrg).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        org: 'acme',
        name: 'org-repo',
        private: false,
        auto_init: true,
      },
    })
    await expect(response.json()).resolves.toEqual({
      repository: {
        owner: 'acme',
        repo: 'org-repo',
        full_name: 'acme/org-repo',
        description: 'A brand new repository',
        private: true,
        html_url: 'https://github.com/acme/org-repo',
      },
    })
  })

  it('returns 403 when the user lacks permissions on the org', async () => {
    const error = Object.assign(new Error('must have admin rights'), { status: 403 })
    createGithubRepositoryForOrg.mockRejectedValue(error)

    const response = await request('/orgs/acme/repos', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: 'org-repo', private: false }),
    })

    expect(response.status).toBe(403)
    await expect(response.json()).resolves.toEqual({
      error: 'must have admin rights',
    })
  })
})
