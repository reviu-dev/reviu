import { afterEach, describe, expect, it, vi } from 'vitest'

import { logger } from '../../lib/logger.js'
import {
  fetchGithubPullRequestCommentsAllPages,
  fetchGithubRepositoryIssueCommentsAllPages,
} from './service.js'

const { requestMock } = vi.hoisted(() => ({
  requestMock: vi.fn(),
}))

vi.mock('@octokit/request', () => ({
  request: requestMock,
}))

describe('github service pagination', () => {
  afterEach(() => {
    requestMock.mockReset()
    vi.restoreAllMocks()
  })

  it('reuses the initial page when continuing PR comment pagination', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: [
        { id: 3, body: 'page-2-comment' },
      ],
    })

    const result = await fetchGithubPullRequestCommentsAllPages({
      token: 'github-token',
      params: {
        owner: 'openai',
        repo: 'reviu',
        pull_number: 42,
      },
      perPage: 2,
      initialPageItems: [
        { id: 1, body: 'page-1-comment-a' } as never,
        { id: 2, body: 'page-1-comment-b' } as never,
      ],
    })

    expect(requestMock).toHaveBeenCalledTimes(1)
    expect(requestMock).toHaveBeenCalledWith(
      'GET /repos/{owner}/{repo}/pulls/{pull_number}/comments',
      expect.objectContaining({
        owner: 'openai',
        repo: 'reviu',
        pull_number: 42,
        per_page: 2,
        page: 2,
      }),
    )

    expect(result).toEqual({
      items: [
        { id: 1, body: 'page-1-comment-a' },
        { id: 2, body: 'page-1-comment-b' },
        { id: 3, body: 'page-2-comment' },
      ],
      pageCount: 2,
      itemCount: 3,
      truncated: false,
    })
  })

  it('marks issue comment pagination as truncated when the configured cap is reached', async () => {
    const loggerWarnSpy = vi.spyOn(logger, 'warn').mockImplementation(() => undefined)

    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: [
        { id: 1, body: 'comment-1' },
        { id: 2, body: 'comment-2' },
      ],
    })

    const result = await fetchGithubRepositoryIssueCommentsAllPages({
      token: 'github-token',
      params: {
        owner: 'openai',
        repo: 'reviu',
        issue_number: 42,
      },
      perPage: 2,
      maxPages: 1,
    })

    expect(result).toEqual({
      items: [
        { id: 1, body: 'comment-1' },
        { id: 2, body: 'comment-2' },
      ],
      pageCount: 1,
      itemCount: 2,
      truncated: true,
    })

    expect(loggerWarnSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        itemCount: 2,
        maxItems: 1000,
        maxPages: 1,
        pageCount: 1,
      }),
      'GitHub paginated collection was truncated at configured limits',
    )
  })
})
