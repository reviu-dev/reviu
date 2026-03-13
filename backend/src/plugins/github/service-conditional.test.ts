import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  fetchGithubCommitConditionally,
  fetchGithubPullRequestCommentsConditionally,
  fetchGithubPullRequestsConditionally,
  fetchGithubRepositoryContentConditionally,
} from './service.js'

const { requestMock } = vi.hoisted(() => ({
  requestMock: vi.fn(),
}))

vi.mock('@octokit/request', () => ({
  request: requestMock,
}))

describe('github service conditional requests', () => {
  afterEach(() => {
    requestMock.mockReset()
  })

  it('forwards validators and returns fresh data for pull request lists', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {
        'etag': '"prs-etag"',
        'last-modified': 'Fri, 13 Mar 2026 18:00:00 GMT',
      },
      data: [],
    })

    const result = await fetchGithubPullRequestsConditionally({
      token: 'github-token',
      params: {
        owner: 'openai',
        repo: 'reviu',
        state: 'all',
        sort: 'updated',
        direction: 'desc',
        per_page: 20,
      },
      etag: '"cached-etag"',
      lastModified: 'Fri, 13 Mar 2026 17:00:00 GMT',
    })

    expect(requestMock).toHaveBeenCalledWith(
      'GET /repos/{owner}/{repo}/pulls',
      expect.objectContaining({
        owner: 'openai',
        repo: 'reviu',
        state: 'all',
        headers: expect.objectContaining({
          'authorization': 'Bearer github-token',
          'if-none-match': '"cached-etag"',
          'if-modified-since': 'Fri, 13 Mar 2026 17:00:00 GMT',
        }),
      }),
    )

    expect(result).toEqual({
      data: [],
      notModified: false,
      etag: '"prs-etag"',
      lastModified: 'Fri, 13 Mar 2026 18:00:00 GMT',
    })
  })

  it('handles 304 responses for repository content while preserving the raw accept header', async () => {
    requestMock.mockRejectedValueOnce({
      status: 304,
      response: {
        headers: {
          'etag': '"content-etag"',
          'last-modified': 'Fri, 13 Mar 2026 18:10:00 GMT',
        },
      },
    })

    const result = await fetchGithubRepositoryContentConditionally({
      token: 'github-token',
      params: {
        owner: 'openai',
        repo: 'reviu',
        path: 'README.md',
        ref: 'main',
      },
      etag: '"cached-content-etag"',
    })

    expect(requestMock).toHaveBeenCalledWith(
      'GET /repos/{owner}/{repo}/contents/{path}',
      expect.objectContaining({
        owner: 'openai',
        repo: 'reviu',
        path: 'README.md',
        ref: 'main',
        headers: expect.objectContaining({
          'authorization': 'Bearer github-token',
          'accept': 'application/vnd.github.raw+json',
          'if-none-match': '"cached-content-etag"',
        }),
      }),
    )

    expect(result).toEqual({
      data: null,
      notModified: true,
      etag: '"content-etag"',
      lastModified: 'Fri, 13 Mar 2026 18:10:00 GMT',
    })
  })

  it('forwards validators for pull request review comments', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {
        etag: '"review-comments-etag"',
      },
      data: [],
    })

    const result = await fetchGithubPullRequestCommentsConditionally({
      token: 'github-token',
      params: {
        owner: 'openai',
        repo: 'reviu',
        pull_number: 42,
        per_page: 100,
      },
      etag: '"cached-review-comments-etag"',
    })

    expect(requestMock).toHaveBeenCalledWith(
      'GET /repos/{owner}/{repo}/pulls/{pull_number}/comments',
      expect.objectContaining({
        owner: 'openai',
        repo: 'reviu',
        pull_number: 42,
        headers: expect.objectContaining({
          'authorization': 'Bearer github-token',
          'if-none-match': '"cached-review-comments-etag"',
        }),
      }),
    )

    expect(result).toEqual({
      data: [],
      notModified: false,
      etag: '"review-comments-etag"',
      lastModified: undefined,
    })
  })

  it('returns not modified for commit validators', async () => {
    requestMock.mockRejectedValueOnce({
      status: 304,
      response: {
        headers: {
          etag: '"commit-etag"',
        },
      },
    })

    const result = await fetchGithubCommitConditionally({
      token: 'github-token',
      params: {
        owner: 'openai',
        repo: 'reviu',
        ref: '0123456789abcdef0123456789abcdef01234567',
      },
      etag: '"cached-commit-etag"',
    })

    expect(requestMock).toHaveBeenCalledWith(
      'GET /repos/{owner}/{repo}/commits/{ref}',
      expect.objectContaining({
        owner: 'openai',
        repo: 'reviu',
        ref: '0123456789abcdef0123456789abcdef01234567',
        headers: expect.objectContaining({
          'authorization': 'Bearer github-token',
          'if-none-match': '"cached-commit-etag"',
        }),
      }),
    )

    expect(result).toEqual({
      data: null,
      notModified: true,
      etag: '"commit-etag"',
      lastModified: undefined,
    })
  })
})
