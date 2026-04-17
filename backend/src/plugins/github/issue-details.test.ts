import { describe, expect, it } from 'vitest'
import {
  buildGithubIssueDetailsValidators,
  GITHUB_ISSUE_DETAILS_COMMENTS_VALIDATOR_KEY,
  GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY,
  mergeGithubIssueDetailsPayload,
} from './issue-details.js'

describe('github issue details helpers', () => {
  it('keeps cached comments when only the issue payload changed', () => {
    const payload = mergeGithubIssueDetailsPayload({
      owner: 'openai',
      repo: 'reviu',
      cachedPayload: {
        node_id: 'I_1',
        reactions: [],
        id: 1,
        number: 42,
        title: 'Old title',
        body: 'Old body',
        state: 'open',
        state_reason: null,
        created_at: '2026-03-13T18:00:00Z',
        updated_at: '2026-03-13T18:00:00Z',
        closed_at: null,
        labels: [],
        comments: [
          {
            node_id: 'IC_9001',
            reactions: [],
            id: 9001,
            body: 'Existing comment',
            created_at: '2026-03-13T18:01:00Z',
            updated_at: '2026-03-13T18:01:00Z',
            user: {
              login: 'octocat',
              avatar_url: 'https://example.com/octocat.png',
            },
          },
        ],
        user: {
          login: 'octocat',
          avatar_url: 'https://example.com/octocat.png',
        },
        repository: {
          owner: 'openai',
          repo: 'reviu',
        },
      },
      issue: {
        node_id: 'I_1',
        id: 1,
        number: 42,
        title: 'New title',
        body: 'New body',
        state: 'open',
        state_reason: null,
        created_at: '2026-03-13T18:00:00Z',
        updated_at: '2026-03-13T18:05:00Z',
        closed_at: null,
        labels: [],
        user: {
          login: 'octocat',
          avatar_url: 'https://example.com/octocat.png',
        },
      } as never,
      issueComments: null,
    })

    expect(payload.title).toBe('New title')
    expect(payload.body).toBe('New body')
    expect(payload.comments).toEqual([
      expect.objectContaining({
        id: 9001,
        body: 'Existing comment',
      }),
    ])
  })

  it('updates only comments when the issue payload was not modified', () => {
    const payload = mergeGithubIssueDetailsPayload({
      owner: 'openai',
      repo: 'reviu',
      cachedPayload: {
        node_id: 'I_1',
        reactions: [],
        id: 1,
        number: 42,
        title: 'Same title',
        body: 'Same body',
        state: 'open',
        state_reason: null,
        created_at: '2026-03-13T18:00:00Z',
        updated_at: '2026-03-13T18:00:00Z',
        closed_at: null,
        labels: [],
        comments: [],
        user: {
          login: 'octocat',
          avatar_url: 'https://example.com/octocat.png',
        },
        repository: {
          owner: 'openai',
          repo: 'reviu',
        },
      },
      issue: null,
      issueComments: [
        {
          node_id: 'IC_9002',
          id: 9002,
          body: 'New comment',
          created_at: '2026-03-13T18:06:00Z',
          updated_at: '2026-03-13T18:06:00Z',
          user: {
            login: 'octocat',
            avatar_url: 'https://example.com/octocat.png',
          },
        },
      ] as never,
    })

    expect(payload.title).toBe('Same title')
    expect(payload.comments).toEqual([
      expect.objectContaining({
        id: 9002,
        body: 'New comment',
      }),
    ])
  })

  it('builds named validators for issue details aggregate entries', () => {
    expect(buildGithubIssueDetailsValidators({
      issue: {
        etag: '"issue-v1"',
      },
      issueComments: {
        etag: '"comments-v1"',
      },
    })).toEqual({
      [GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY]: {
        etag: '"issue-v1"',
      },
      [GITHUB_ISSUE_DETAILS_COMMENTS_VALIDATOR_KEY]: {
        etag: '"comments-v1"',
      },
    })
  })
})
