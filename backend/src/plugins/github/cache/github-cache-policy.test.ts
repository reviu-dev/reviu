import { describe, expect, it } from 'vitest'

import {
  createGithubNotificationsCachePolicy,
  createGithubPullRequestCommentsCachePolicy,
  createGithubPullRequestCommitsCachePolicy,
  createGithubPullRequestDetailsCachePolicy,
  createGithubPullRequestFilesCachePolicy,
  createGithubPullRequestIssueCommentsCachePolicy,
  createGithubPullRequestSearchCachePolicy,
  createGithubRepositoryReadmeCachePolicy,
  getGithubIssueMutationTags,
  getGithubPullRequestMutationTags,
} from './github-cache-policy.js'

describe('github cache policy', () => {
  it('builds the notifications cache policy for a viewer', () => {
    expect(createGithubNotificationsCachePolicy('user-1')).toEqual({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'notifications',
      ttlMs: 15_000,
      staleMs: 60_000,
      tags: ['viewer:user-1:notifications'],
    })
  })

  it('builds the pull request search cache policy with broad and variant tags', () => {
    expect(createGithubPullRequestSearchCachePolicy('user-1', 'need-review')).toEqual({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:need-review-pull-requests',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: [
        'viewer:user-1:pr-search',
        'viewer:user-1:pr-search:need-review',
      ],
    })
  })

  it('builds the pull request details cache policy', () => {
    expect(createGithubPullRequestDetailsCachePolicy('user-1', 'OpenAI', 'Reviu', 42)).toEqual({
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
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:openai/reviu:42:commits',
      ttlMs: 60_000,
      staleMs: 600_000,
      tags: ['pull-request:openai/reviu:42:commits'],
    })
  })

  it('builds the pull request issue comments cache policy', () => {
    expect(createGithubPullRequestIssueCommentsCachePolicy('user-1', 'OpenAI', 'Reviu', 42)).toEqual({
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
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:openai/reviu:42:comments',
      ttlMs: 15_000,
      staleMs: 120_000,
      tags: ['pull-request:openai/reviu:42:comments'],
    })
  })

  it('builds a ref-specific readme cache policy', () => {
    expect(createGithubRepositoryReadmeCachePolicy('user-1', 'OpenAI', 'Reviu', 'feature/cache')).toEqual({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'repo:openai/reviu:readme:ref:feature%2Fcache',
      ttlMs: 300_000,
      staleMs: 1_800_000,
      tags: ['repo:openai/reviu:readme'],
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
