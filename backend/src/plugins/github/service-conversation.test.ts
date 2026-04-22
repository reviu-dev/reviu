import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  addGithubReactionGraphql,
  fetchGithubIssueDetailsGraphql,
  fetchGithubPullRequestConversationGraphql,
  removeGithubReactionGraphql,
} from './service.js'

const { requestMock } = vi.hoisted(() => ({
  requestMock: vi.fn(),
}))

vi.mock('@octokit/request', () => ({
  request: requestMock,
}))

const emptyPageInfo = {
  hasNextPage: false,
  endCursor: null,
}

const thumbsUpReactionGroup = {
  content: 'THUMBS_UP',
  viewerHasReacted: true,
  reactors: {
    totalCount: 2,
  },
}

describe('github pull request conversation service', () => {
  afterEach(() => {
    requestMock.mockReset()
  })

  it('maps GraphQL pull request conversation nodes to the desktop DTO', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: {
        data: {
          repository: {
            pullRequest: {
              id: 'PR_kwDOExample',
              reactionGroups: [thumbsUpReactionGroup],
              comments: {
                nodes: [{
                  id: 'IC_kwDOExample',
                  fullDatabaseId: '11',
                  body: 'Can you add tests?',
                  createdAt: '2026-02-28T10:00:00Z',
                  updatedAt: '2026-02-28T10:05:00Z',
                  reactionGroups: [thumbsUpReactionGroup],
                  author: {
                    login: 'octocat',
                    avatarUrl: 'https://avatars.githubusercontent.com/u/1?v=4',
                  },
                }],
                pageInfo: emptyPageInfo,
              },
              reviews: {
                nodes: [{
                  id: 'PRR_kwDOExample',
                  databaseId: 123,
                  body: 'Looks good',
                  state: 'APPROVED',
                  submittedAt: '2026-02-28T12:00:00Z',
                  commit: {
                    oid: '1111111111111111111111111111111111111111',
                  },
                  url: 'https://github.com/acme/widget/pull/42#pullrequestreview-123',
                  reactionGroups: [],
                  author: {
                    login: 'reviewer',
                    avatarUrl: 'https://avatars.githubusercontent.com/u/2?v=4',
                  },
                }],
                pageInfo: emptyPageInfo,
              },
              reviewThreads: {
                nodes: [{
                  id: 'PRRT_kwDOExample',
                  isOutdated: false,
                  path: 'src/main.rs',
                  line: 5,
                  originalLine: 4,
                  startLine: null,
                  originalStartLine: null,
                  diffSide: 'RIGHT',
                  startDiffSide: null,
                  comments: {
                    nodes: [{
                      id: 'PRRC_kwDOExample',
                      fullDatabaseId: '1001',
                      diffHunk: '@@ -1 +1 @@',
                      path: 'src/main.rs',
                      position: 1,
                      originalPosition: 1,
                      commit: {
                        oid: 'head123',
                      },
                      originalCommit: {
                        oid: 'base123',
                      },
                      pullRequestReview: {
                        id: 'PRR_kwDOExample',
                        databaseId: 123,
                      },
                      replyTo: null,
                      author: {
                        login: 'octocat',
                        avatarUrl: 'https://avatars.githubusercontent.com/u/1?v=4',
                      },
                      body: 'Looks good',
                      createdAt: '2026-02-15T12:00:00Z',
                      updatedAt: '2026-02-15T12:01:00Z',
                      reactionGroups: [],
                      startLine: null,
                      originalStartLine: null,
                      line: 5,
                      originalLine: 4,
                    }],
                    pageInfo: emptyPageInfo,
                  },
                }],
                pageInfo: emptyPageInfo,
              },
            },
          },
        },
      },
    })

    const conversation = await fetchGithubPullRequestConversationGraphql({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
      pullNumber: 42,
    })

    expect(requestMock).toHaveBeenCalledWith(
      'POST /graphql',
      expect.objectContaining({
        headers: expect.objectContaining({
          authorization: 'Bearer github-token',
        }),
        variables: {
          owner: 'acme',
          name: 'widget',
          number: 42,
        },
      }),
    )
    expect(conversation).toEqual({
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
        user: {
          login: 'octocat',
          avatar_url: 'https://avatars.githubusercontent.com/u/1?v=4',
        },
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
        user: {
          login: 'reviewer',
          avatar_url: 'https://avatars.githubusercontent.com/u/2?v=4',
        },
      }],
      review_comments: [{
        node_id: 'PRRC_kwDOExample',
        reactions: [],
        is_outdated: false,
        id: 1001,
        pull_request_review_id: 123,
        diff_hunk: '@@ -1 +1 @@',
        path: 'src/main.rs',
        position: 1,
        original_position: 1,
        commit_id: 'head123',
        original_commit_id: 'base123',
        in_reply_to_id: undefined,
        user: {
          login: 'octocat',
          avatar_url: 'https://avatars.githubusercontent.com/u/1?v=4',
        },
        body: 'Looks good',
        created_at: '2026-02-15T12:00:00Z',
        updated_at: '2026-02-15T12:01:00Z',
        start_line: null,
        original_start_line: null,
        start_side: undefined,
        line: 5,
        original_line: 4,
        side: 'RIGHT',
      }],
    })
  })

  it('adds a GraphQL reaction and returns updated reaction groups', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: {
        data: {
          addReaction: {
            reactionGroups: [thumbsUpReactionGroup],
          },
        },
      },
    })

    const reactions = await addGithubReactionGraphql({
      token: 'github-token',
      subjectId: 'IC_kwDOExample',
      content: 'THUMBS_UP',
    })

    expect(requestMock).toHaveBeenCalledWith(
      'POST /graphql',
      expect.objectContaining({
        variables: {
          subjectId: 'IC_kwDOExample',
          content: 'THUMBS_UP',
        },
      }),
    )
    expect(reactions).toEqual([{
      content: 'THUMBS_UP',
      count: 2,
      viewer_has_reacted: true,
    }])
  })

  it('maps GitHub OAuth app access restrictions to a forbidden error', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: {
        data: null,
        errors: [{
          message: 'Although you appear to have the correct authorization credentials, the `openai` organization has enabled OAuth App access restrictions, meaning that data access to third-parties is limited.',
        }],
      },
    })

    await expect(addGithubReactionGraphql({
      token: 'github-token',
      subjectId: 'I_kwDOExample',
      content: 'THUMBS_UP',
    })).rejects.toMatchObject({
      status: 403,
      message: 'The openai organization restricts OAuth app access. Ask an organization owner to approve Reviu, then try again.',
    })
  })

  it('removes a GraphQL reaction and returns updated reaction groups', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: {
        data: {
          removeReaction: {
            reactionGroups: [],
          },
        },
      },
    })

    const reactions = await removeGithubReactionGraphql({
      token: 'github-token',
      subjectId: 'IC_kwDOExample',
      content: 'THUMBS_UP',
    })

    expect(requestMock).toHaveBeenCalledWith(
      'POST /graphql',
      expect.objectContaining({
        variables: {
          subjectId: 'IC_kwDOExample',
          content: 'THUMBS_UP',
        },
      }),
    )
    expect(reactions).toEqual([])
  })

  it('maps GraphQL issue details and reaction groups to the desktop DTO', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: {
        data: {
          repository: {
            issue: {
              id: 'I_kwDOExample',
              fullDatabaseId: '501',
              reactionGroups: [thumbsUpReactionGroup],
              number: 77,
              title: 'Fix auth race condition',
              body: 'Issue body',
              state: 'CLOSED',
              stateReason: 'COMPLETED',
              createdAt: '2026-02-20T08:00:00Z',
              updatedAt: '2026-02-21T09:30:00Z',
              closedAt: '2026-02-21T09:30:00Z',
              author: {
                login: 'octocat',
                avatarUrl: 'https://avatars.githubusercontent.com/u/1?v=4',
              },
              labels: {
                nodes: [{
                  name: 'bug',
                  color: 'f29513',
                }],
              },
              repository: {
                owner: {
                  login: 'acme',
                },
                name: 'widget',
              },
              comments: {
                nodes: [{
                  id: 'IC_kwDOIssue',
                  databaseId: 9001,
                  reactionGroups: [thumbsUpReactionGroup],
                  body: 'Looks good',
                  createdAt: '2026-02-20T10:00:00Z',
                  updatedAt: '2026-02-20T10:05:00Z',
                  author: {
                    login: 'reviewer',
                    avatarUrl: 'https://avatars.githubusercontent.com/u/2?v=4',
                  },
                }],
                pageInfo: emptyPageInfo,
              },
            },
          },
        },
      },
    })

    const issue = await fetchGithubIssueDetailsGraphql({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
      issueNumber: 77,
    })

    expect(requestMock).toHaveBeenCalledWith(
      'POST /graphql',
      expect.objectContaining({
        variables: {
          owner: 'acme',
          name: 'widget',
          number: 77,
        },
      }),
    )
    expect(issue).toEqual({
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
        reactions: [{
          content: 'THUMBS_UP',
          count: 2,
          viewer_has_reacted: true,
        }],
        id: 9001,
        body: 'Looks good',
        created_at: '2026-02-20T10:00:00Z',
        updated_at: '2026-02-20T10:05:00Z',
        user: {
          login: 'reviewer',
          avatar_url: 'https://avatars.githubusercontent.com/u/2?v=4',
        },
      }],
      user: {
        login: 'octocat',
        avatar_url: 'https://avatars.githubusercontent.com/u/1?v=4',
      },
      repository: {
        owner: 'acme',
        repo: 'widget',
      },
    })
  })
})
