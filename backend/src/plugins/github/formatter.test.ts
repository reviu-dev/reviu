import { describe, expect, it } from 'vitest'
import {
  mapGithubGraphqlCommitAuthors,
  mapGithubGraphqlPullRequest,
  mapGithubGraphqlPullRequestCommit,
  mapGithubPullRequest,
} from './formatter.js'

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

describe('github formatter commit authors', () => {
  it('maps GraphQL commit author identities', () => {
    expect(
      mapGithubGraphqlCommitAuthors([
        {
          name: 'Octo Cat',
          email: 'octocat@example.com',
          user: {
            login: 'octocat',
            avatarUrl: 'https://example.com/octocat.png',
          },
        },
        {
          name: 'Co Author',
          email: 'coauthor@example.com',
          user: null,
        },
        {
          name: 'Octo Cat',
          email: 'octocat@example.com',
          user: null,
        },
      ]),
    ).toEqual([
      {
        name: 'Octo Cat',
        email: 'octocat@example.com',
        login: 'octocat',
        avatar_url: 'https://example.com/octocat.png',
      },
      {
        name: 'Co Author',
        email: 'coauthor@example.com',
        login: null,
        avatar_url: null,
      },
    ])
  })

  it('maps GraphQL pull request commit authors', () => {
    const commit = mapGithubGraphqlPullRequestCommit({
      commit: {
        oid: 'abc123',
        message: 'feat: use graphql commits',
        authoredDate: '2026-04-20T12:00:00Z',
        committedDate: '2026-04-20T12:05:00Z',
        parents: {
          nodes: [{ oid: 'parent123' }],
        },
        author: {
          name: 'Octo Cat',
          email: 'octocat@example.com',
          user: {
            login: 'octocat',
            avatarUrl: 'https://example.com/octocat.png',
          },
        },
        committer: {
          user: {
            login: 'web-flow',
            avatarUrl: 'https://example.com/web-flow.png',
          },
        },
        authors: {
          nodes: [
            {
              name: 'Octo Cat',
              email: 'octocat@example.com',
              user: {
                login: 'octocat',
                avatarUrl: 'https://example.com/octocat.png',
              },
            },
            {
              name: 'Co Author',
              email: 'coauthor@example.com',
              user: null,
            },
          ],
        },
      },
    })

    expect(commit).toEqual({
      sha: 'abc123',
      message: 'feat: use graphql commits',
      authored_at: '2026-04-20T12:00:00Z',
      committed_at: '2026-04-20T12:05:00Z',
      parent_sha: 'parent123',
      author: {
        login: 'octocat',
        avatar_url: 'https://example.com/octocat.png',
      },
      committer: {
        login: 'web-flow',
        avatar_url: 'https://example.com/web-flow.png',
      },
      authors: [
        {
          name: 'Octo Cat',
          email: 'octocat@example.com',
          login: 'octocat',
          avatar_url: 'https://example.com/octocat.png',
        },
        {
          name: 'Co Author',
          email: 'coauthor@example.com',
          login: null,
          avatar_url: null,
        },
      ],
    })
  })
})
