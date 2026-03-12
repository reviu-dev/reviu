import { describe, expect, it } from 'vitest'

import {
  createGithubNotificationsCachePolicy,
  createGithubPullRequestSearchCachePolicy,
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
