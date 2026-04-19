import { describe, expect, it } from 'vitest'

import {
  createGithubNotificationsCachePolicy,
  createGithubPullRequestCommentsCachePolicy,
  createGithubPullRequestCommitsCachePolicy,
  createGithubPullRequestConversationCachePolicy,
  createGithubPullRequestDetailsCachePolicy,
  createGithubPullRequestFilesCachePolicy,
  createGithubPullRequestIssueCommentsCachePolicy,
  createGithubPullRequestSearchCachePolicy,
  createGithubRepositoryCommitCachePolicy,
  createGithubRepositoryFileCachePolicy,
  createGithubRepositoryIssueDetailsCachePolicy,
  createGithubRepositoryIssuesCachePolicy,
  createGithubRepositoryPullRequestsCachePolicy,
  createGithubRepositoryReadmeCachePolicy,
  createGithubRepositoryTreeCachePolicy,
  getGithubIssueMutationTags,
  getGithubPullRequestMutationTags,
  withGithubPublicScope,
} from './github-cache-policy.js'

describe('github cache policy', () => {
  it('builds the notifications cache policy for a viewer', () => {
    expect(createGithubNotificationsCachePolicy('user-1')).toEqual({
      operation: 'viewer.notifications',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'notifications',
      ttlMs: 15_000,
      staleMs: 60_000,
      tags: ['viewer:user-1:notifications'],
    })
  })

  it('builds the pull request search cache policy with a normalized cache key', () => {
    expect(createGithubPullRequestSearchCachePolicy('user-1', '{"authors":["@me"]}')).toEqual({
      operation: 'viewer.pull_requests.search',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:pull-requests:%7B%22authors%22%3A%5B%22%40me%22%5D%7D',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1:pr-search'],
    })
  })

  it('builds the pull request details cache policy', () => {
    expect(createGithubPullRequestDetailsCachePolicy('user-1', 'OpenAI', 'Reviu', 42)).toEqual({
      operation: 'pull_request.details',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:openai/reviu:42:details',
      ttlMs: 20_000,
      staleMs: 120_000,
      tags: ['pull-request:openai/reviu:42'],
    })
  })

  it('builds a commit-specific files cache policy with a longer ttl', () => {
    expect(createGithubPullRequestFilesCachePolicy('user-1', 'OpenAI', 'Reviu', 42, 'ABC123')).toEqual({
      operation: 'pull_request.files.commit',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:openai/reviu:42:files:commit:ABC123',
      ttlMs: 600_000,
      staleMs: 3_600_000,
      tags: ['pull-request:openai/reviu:42:files'],
    })
  })

  it('builds the pull request commits cache policy', () => {
    expect(createGithubPullRequestCommitsCachePolicy('user-1', 'OpenAI', 'Reviu', 42)).toEqual({
      operation: 'pull_request.commits',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:openai/reviu:42:commits',
      ttlMs: 60_000,
      staleMs: 600_000,
      tags: ['pull-request:openai/reviu:42:commits'],
    })
  })

  it('builds the repository commit cache policy', () => {
    expect(createGithubRepositoryCommitCachePolicy('user-1', 'OpenAI', 'Reviu', 'ABC123')).toEqual({
      operation: 'repository.commit',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:commit:ABC123',
      ttlMs: 86_400_000,
      staleMs: 604_800_000,
      tags: ['repo:openai/reviu:commit:ABC123'],
    })
  })

  it('builds the pull request issue comments cache policy', () => {
    expect(createGithubPullRequestIssueCommentsCachePolicy('user-1', 'OpenAI', 'Reviu', 42)).toEqual({
      operation: 'pull_request.issue_comments',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:openai/reviu:42:issue-comments',
      ttlMs: 15_000,
      staleMs: 120_000,
      tags: ['issue:openai/reviu:42:comments'],
    })
  })

  it('builds the pull request review comments cache policy', () => {
    expect(createGithubPullRequestCommentsCachePolicy('user-1', 'OpenAI', 'Reviu', 42)).toEqual({
      operation: 'pull_request.comments',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:openai/reviu:42:comments',
      ttlMs: 15_000,
      staleMs: 120_000,
      tags: ['pull-request:openai/reviu:42:comments'],
    })
  })

  it('builds the pull request conversation cache policy with all conversation tags', () => {
    expect(createGithubPullRequestConversationCachePolicy('user-1', 'OpenAI', 'Reviu', 42)).toEqual({
      operation: 'pull_request.conversation',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:openai/reviu:42:conversation',
      ttlMs: 15_000,
      staleMs: 120_000,
      tags: [
        'pull-request:openai/reviu:42',
        'issue:openai/reviu:42:comments',
        'pull-request:openai/reviu:42:comments',
        'pull-request:openai/reviu:42:reviews',
      ],
    })
  })

  it('builds a ref-specific readme cache policy', () => {
    expect(createGithubRepositoryReadmeCachePolicy('user-1', 'OpenAI', 'Reviu', 'feature/cache')).toEqual({
      operation: 'repository.readme.ref',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:readme:ref:feature%2Fcache',
      ttlMs: 300_000,
      staleMs: 1_800_000,
      tags: ['repo:openai/reviu:readme'],
    })
  })

  it('builds the repository pull requests cache policy', () => {
    expect(createGithubRepositoryPullRequestsCachePolicy('user-1', 'OpenAI', 'Reviu')).toEqual({
      operation: 'repository.pull_requests',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:pull-requests',
      ttlMs: 30_000,
      staleMs: 300_000,
      tags: ['repo:openai/reviu:pull-requests'],
    })
  })

  it('builds a filtered repository pull requests cache policy', () => {
    expect(
      createGithubRepositoryPullRequestsCachePolicy(
        'user-1',
        'OpenAI',
        'Reviu',
        '{"labels":["bug"],"sort":"updated_desc"}',
      ),
    ).toMatchObject({
      operation: 'repository.pull_requests',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:pull-requests:%7B%22labels%22%3A%5B%22bug%22%5D%2C%22sort%22%3A%22updated_desc%22%7D',
      tags: ['repo:openai/reviu:pull-requests'],
    })
  })

  it('builds the repository issues cache policy', () => {
    expect(createGithubRepositoryIssuesCachePolicy('user-1', 'OpenAI', 'Reviu')).toEqual({
      operation: 'repository.issues',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:issues',
      ttlMs: 30_000,
      staleMs: 300_000,
      tags: ['repo:openai/reviu:issues'],
    })
  })

  it('builds the repository issues cache policy with state', () => {
    expect(createGithubRepositoryIssuesCachePolicy('user-1', 'OpenAI', 'Reviu', 'open')).toEqual({
      operation: 'repository.issues',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:issues:open',
      ttlMs: 30_000,
      staleMs: 300_000,
      tags: ['repo:openai/reviu:issues'],
    })
  })

  it('builds the repository issue details cache policy with issue and comment tags', () => {
    expect(createGithubRepositoryIssueDetailsCachePolicy('user-1', 'OpenAI', 'Reviu', 7)).toEqual({
      operation: 'repository.issue_details',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:issue:7',
      ttlMs: 15_000,
      staleMs: 120_000,
      tags: ['issue:openai/reviu:7', 'issue:openai/reviu:7:comments'],
    })
  })

  it('builds the repository tree cache policy', () => {
    expect(createGithubRepositoryTreeCachePolicy('user-1', 'OpenAI', 'Reviu', 'abc123', '1')).toEqual({
      operation: 'repository.tree.recursive',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:tree:abc123:recursive:1',
      ttlMs: 600_000,
      staleMs: 86_400_000,
      tags: ['repo:openai/reviu:tree:abc123'],
    })
  })

  it('builds a long-lived file cache policy for blob sha refs', () => {
    expect(createGithubRepositoryFileCachePolicy('user-1', 'OpenAI', 'Reviu', 'src/main.ts', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')).toEqual({
      operation: 'repository.file.blob',
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:file:blob:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:path:src%2Fmain.ts',
      ttlMs: 86_400_000,
      staleMs: 604_800_000,
      tags: ['repo:openai/reviu:file:blob:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'],
    })
  })

  it('promotes a viewer cache policy to public scope without changing the resource identity', () => {
    expect(withGithubPublicScope(
      createGithubRepositoryReadmeCachePolicy('user-1', 'OpenAI', 'Reviu'),
    )).toEqual({
      operation: 'repository.readme',
      scope: 'public',
      scopeId: undefined,
      resourceKey: 'repo:openai/reviu:readme',
      ttlMs: 120_000,
      staleMs: 600_000,
      tags: ['repo:openai/reviu:readme'],
    })
  })

  it('promotes a pull request cache policy to public scope without changing pull request tags', () => {
    expect(withGithubPublicScope(
      createGithubPullRequestDetailsCachePolicy('user-1', 'OpenAI', 'Reviu', 42),
    )).toEqual({
      operation: 'pull_request.details',
      scope: 'public',
      scopeId: undefined,
      resourceKey: 'pull-request:openai/reviu:42:details',
      ttlMs: 20_000,
      staleMs: 120_000,
      tags: ['pull-request:openai/reviu:42'],
    })
  })

  it('builds normalized pull request invalidation tags', () => {
    expect(getGithubPullRequestMutationTags({
      userId: 'user-1',
      owner: 'OpenAI',
      repo: 'Reviu',
      pullNumber: 42,
      includeComments: true,
      includeReviews: true,
    })).toEqual([
      'viewer:user-1:pr-search',
      'repo:openai/reviu:pull-requests',
      'pull-request:openai/reviu:42',
      'pull-request:openai/reviu:42:comments',
      'pull-request:openai/reviu:42:reviews',
    ])
  })

  it('builds issue invalidation tags only for issue resources', () => {
    expect(getGithubIssueMutationTags({
      owner: 'OpenAI',
      repo: 'Reviu',
      issueNumber: 7,
      includeComments: true,
    })).toEqual([
      'repo:openai/reviu:issues',
      'issue:openai/reviu:7',
      'issue:openai/reviu:7:comments',
    ])
  })
})
