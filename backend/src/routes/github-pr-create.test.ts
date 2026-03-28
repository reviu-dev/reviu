import type { CreatePullRequestResponse } from '../plugins/github/types.js'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const createGithubPullRequest = vi.fn()

vi.mock('../middlewares/auth.js', async () => {
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

vi.mock('../plugins/github/service.js', async () => {
  const actual = await vi.importActual<typeof import('../plugins/github/service.js')>(
    '../plugins/github/service.js',
  )

  return {
    ...actual,
    createGithubPullRequest,
  }
})

vi.mock('../plugins/github/cache/github-cache-runtime.js', () => ({
  githubCache: {
    getOrLoad: vi.fn(),
    invalidateTags: vi.fn(),
    prime: vi.fn(),
  },
}))

vi.mock('../plugins/github/cache/github-repository-visibility-runtime.js', () => ({
  githubRepositoryVisibility: {
    isKnownPublic: vi.fn(),
    clear: vi.fn(),
    markPublic: vi.fn(),
  },
}))

vi.mock('../plugins/github/metrics/github-metrics-context.js', () => ({
  runWithGithubMetricsContext: (_context: unknown, callback: () => Promise<unknown>) => callback(),
}))

function makePullRequest(
  overrides: Partial<CreatePullRequestResponse> = {},
): CreatePullRequestResponse {
  return {
    number: 42,
    state: 'open',
    locked: false,
    title: 'Parser cleanup',
    user: {
      login: 'octocat',
      id: 1,
      node_id: 'user-1',
      avatar_url: 'https://example.com/avatar.png',
      gravatar_id: '',
      url: 'https://api.github.com/users/octocat',
      html_url: 'https://github.com/octocat',
      followers_url: '',
      following_url: '',
      gists_url: '',
      starred_url: '',
      subscriptions_url: '',
      organizations_url: '',
      repos_url: '',
      events_url: '',
      received_events_url: '',
      type: 'User',
      site_admin: false,
    },
    body: 'Body',
    created_at: '2026-03-20T09:00:00Z',
    updated_at: '2026-03-21T10:00:00Z',
    closed_at: null,
    merged_at: null,
    merge_commit_sha: null,
    assignee: null,
    assignees: [],
    requested_reviewers: [],
    requested_teams: [],
    labels: [{ id: 1, node_id: 'label-1', url: '', name: 'bug', color: 'ff0000', default: false, description: null }],
    milestone: null,
    draft: false,
    commits_url: '',
    review_comments_url: '',
    review_comment_url: '',
    comments_url: '',
    statuses_url: '',
    head: {
      label: 'acme:feature/parser',
      ref: 'feature/parser',
      sha: 'head-sha',
      user: null,
      repo: null,
    },
    base: {
      label: 'acme:main',
      ref: 'main',
      sha: 'base-sha',
      user: null,
      repo: {
        id: 2,
        node_id: 'repo-2',
        name: 'widget',
        full_name: 'acme/widget',
        private: false,
        owner: {
          login: 'acme',
          id: 2,
          node_id: 'owner-2',
          avatar_url: 'https://example.com/org.png',
          gravatar_id: '',
          url: 'https://api.github.com/users/acme',
          html_url: 'https://github.com/acme',
          followers_url: '',
          following_url: '',
          gists_url: '',
          starred_url: '',
          subscriptions_url: '',
          organizations_url: '',
          repos_url: '',
          events_url: '',
          received_events_url: '',
          type: 'Organization',
          site_admin: false,
        },
        html_url: 'https://github.com/acme/widget',
        description: null,
        fork: false,
        url: 'https://api.github.com/repos/acme/widget',
        archive_url: '',
        assignees_url: '',
        blobs_url: '',
        branches_url: '',
        collaborators_url: '',
        comments_url: '',
        commits_url: '',
        compare_url: '',
        contents_url: '',
        contributors_url: '',
        deployments_url: '',
        downloads_url: '',
        events_url: '',
        forks_url: '',
        git_commits_url: '',
        git_refs_url: '',
        git_tags_url: '',
        git_url: '',
        issue_comment_url: '',
        issue_events_url: '',
        issues_url: '',
        keys_url: '',
        labels_url: '',
        languages_url: '',
        merges_url: '',
        milestones_url: '',
        notifications_url: '',
        pulls_url: '',
        releases_url: '',
        ssh_url: '',
        stargazers_url: '',
        statuses_url: '',
        subscribers_url: '',
        subscription_url: '',
        tags_url: '',
        teams_url: '',
        trees_url: '',
        clone_url: '',
        mirror_url: null,
        hooks_url: '',
        svn_url: '',
        homepage: null,
        language: null,
        forks_count: 0,
        stargazers_count: 0,
        watchers_count: 0,
        size: 0,
        default_branch: 'main',
        open_issues_count: 0,
        is_template: false,
        topics: [],
        has_issues: true,
        has_projects: true,
        has_wiki: true,
        has_pages: false,
        has_downloads: true,
        archived: false,
        disabled: false,
        visibility: 'public',
        pushed_at: '2026-03-20T09:00:00Z',
        created_at: '2026-03-20T09:00:00Z',
        updated_at: '2026-03-20T09:00:00Z',
        permissions: undefined,
        allow_rebase_merge: true,
        template_repository: null,
        temp_clone_token: null,
        allow_squash_merge: true,
        allow_auto_merge: false,
        delete_branch_on_merge: false,
        allow_update_branch: true,
        use_squash_pr_title_as_default: false,
        squash_merge_commit_message: 'COMMIT_MESSAGES',
        squash_merge_commit_title: 'COMMIT_OR_PR_TITLE',
        merge_commit_message: 'PR_TITLE',
        merge_commit_title: 'MERGE_MESSAGE',
        subscribers_count: 0,
        network_count: 0,
        license: null,
        forks: 0,
        open_issues: 0,
        watchers: 0,
        web_commit_signoff_required: false,
      },
    },
    _links: {
      self: { href: '' },
      html: { href: '' },
      issue: { href: '' },
      comments: { href: '' },
      review_comments: { href: '' },
      review_comment: { href: '' },
      commits: { href: '' },
      statuses: { href: '' },
    },
    author_association: 'OWNER',
    auto_merge: null,
    active_lock_reason: null,
    html_url: 'https://github.com/acme/widget/pull/42',
    id: 42,
    issue_url: '',
    node_id: 'pr-42',
    diff_url: '',
    patch_url: '',
    url: '',
    comments: 2,
    review_comments: 3,
    commits: 1,
    additions: 12,
    deletions: 3,
    changed_files: 2,
    maintainer_can_modify: true,
    rebaseable: true,
    mergeable: true,
    mergeable_state: 'clean',
    merged: false,
    merged_by: null,
  } as unknown as CreatePullRequestResponse
}

async function request(path: string, init?: RequestInit) {
  const { githubRoutes } = await import('./github.js')
  return githubRoutes.request(`http://localhost${path}`, init)
}

describe('github create pull request route', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('creates a pull request for the requested branch', async () => {
    createGithubPullRequest.mockResolvedValue(makePullRequest())

    const response = await request('/repos/acme/widget/pr?branch=feature/parser', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        title: 'Parser cleanup',
        base: 'main',
        body: 'Ready to review',
        draft: true,
      }),
    })

    expect(response.status).toBe(201)
    expect(createGithubPullRequest).toHaveBeenCalledWith({
      token: 'github-token',
      params: {
        owner: 'acme',
        repo: 'widget',
        head: 'acme:feature/parser',
        base: 'main',
        title: 'Parser cleanup',
        body: 'Ready to review',
        draft: true,
      },
    })
    await expect(response.json()).resolves.toEqual({
      pullRequest: expect.objectContaining({
        number: 42,
        title: 'Parser cleanup',
        repository: {
          owner: 'acme',
          repo: 'widget',
        },
      }),
    })
  })

  it('returns 400 when the branch query is missing', async () => {
    const response = await request('/repos/acme/widget/pr', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        title: 'Parser cleanup',
        base: 'main',
      }),
    })

    expect(response.status).toBe(400)
    await expect(response.json()).resolves.toEqual({
      error: 'Missing branch',
    })
    expect(createGithubPullRequest).not.toHaveBeenCalled()
  })

  it('returns 400 when the title is missing', async () => {
    const response = await request('/repos/acme/widget/pr?branch=feature/parser', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        title: '   ',
        base: 'main',
      }),
    })

    expect(response.status).toBe(400)
    await expect(response.json()).resolves.toEqual({
      error: expect.stringContaining('Missing pull request title'),
    })
    expect(createGithubPullRequest).not.toHaveBeenCalled()
  })
})
