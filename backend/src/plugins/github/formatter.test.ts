import { describe, expect, it } from 'vitest'
import { mapGithubGraphqlPullRequest, mapGithubPullRequest } from './formatter.js'

describe('github formatter label colors', () => {
  it('preserves GraphQL pull request label colors', () => {
    const result = mapGithubGraphqlPullRequest({
      number: 7,
      title: 'Fix login issue',
      state: 'OPEN',
      isDraft: false,
      createdAt: '2026-03-20T09:00:00Z',
      updatedAt: '2026-03-20T10:00:00Z',
      closedAt: null,
      mergedAt: null,
      author: {
        __typename: 'User',
        login: 'octocat',
        avatarUrl: 'https://example.com/octocat.png',
      },
      labels: {
        nodes: [
          {
            name: 'bug',
            color: 'f29513',
          },
        ],
      },
      repository: {
        owner: {
          login: 'acme',
        },
        name: 'widget',
      },
      comments: {
        totalCount: 3,
      },
      reviews: {
        totalCount: 2,
      },
    })

    expect(result.pullRequest.labels).toEqual([
      {
        name: 'bug',
        color: 'f29513',
      },
    ])
  })

  it('preserves REST pull request label colors', () => {
    const result = mapGithubPullRequest({
      number: 11,
      title: 'Improve docs',
      state: 'open',
      created_at: '2026-03-20T09:00:00Z',
      closed_at: null,
      merged_at: null,
      updated_at: '2026-03-20T10:00:00Z',
      draft: false,
      labels: [
        {
          name: 'documentation',
          color: '0075ca',
        },
      ],
      user: {
        login: 'octocat',
        avatar_url: 'https://example.com/octocat.png',
        type: 'User',
      },
      base: {
        repo: {
          owner: {
            login: 'acme',
          },
          name: 'widget',
        },
      },
    } as never)

    expect(result.labels).toEqual([
      {
        name: 'documentation',
        color: '0075ca',
      },
    ])
  })
})
