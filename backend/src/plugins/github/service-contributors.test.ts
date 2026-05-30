import { afterEach, describe, expect, it, vi } from 'vitest'

import { fetchGithubRepositoryContributors } from './service.js'

const { requestMock } = vi.hoisted(() => ({
  requestMock: vi.fn(),
}))

vi.mock('@octokit/request', () => ({
  request: requestMock,
}))

describe('fetchGithubRepositoryContributors', () => {
  afterEach(() => {
    requestMock.mockReset()
  })

  it('hits the REST contributors endpoint with anon flag and per_page 100', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: [
        { login: 'alice', avatar_url: 'https://avatars/alice', contributions: 12 },
        { login: 'bob', avatar_url: 'https://avatars/bob', contributions: 3 },
      ],
    })

    const result = await fetchGithubRepositoryContributors({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
    })

    expect(requestMock).toHaveBeenCalledWith(
      'GET /repos/{owner}/{repo}/contributors',
      expect.objectContaining({
        owner: 'acme',
        repo: 'widget',
        per_page: 100,
        anon: 'true',
        headers: expect.objectContaining({
          authorization: 'Bearer github-token',
        }),
      }),
    )
    expect(result).toEqual({
      contributors: [
        { login: 'alice', avatar_url: 'https://avatars/alice' },
        { login: 'bob', avatar_url: 'https://avatars/bob' },
      ],
      total_count: 2,
    })
  })

  it('drops anonymous entries that lack a login or avatar_url', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: [
        { login: 'alice', avatar_url: 'https://avatars/alice' },
        { email: 'ghost@example.com', name: 'Ghost' },
        { login: 'bob' },
      ],
    })

    const result = await fetchGithubRepositoryContributors({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
    })

    expect(result.contributors).toEqual([
      { login: 'alice', avatar_url: 'https://avatars/alice' },
    ])
    expect(result.total_count).toBe(1)
  })

  it('infers total_count from the Link header when paginated', async () => {
    const items = Array.from({ length: 100 }, (_, ix) => ({
      login: `user-${ix}`,
      avatar_url: `https://avatars/${ix}`,
    }))
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {
        link: '<https://api.github.com/repositories/1/contributors?per_page=100&page=2>; rel="next", <https://api.github.com/repositories/1/contributors?per_page=100&page=5>; rel="last"',
      },
      data: items,
    })

    const result = await fetchGithubRepositoryContributors({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
    })

    expect(result.contributors).toHaveLength(100)
    expect(result.total_count).toBe(500)
  })

  it('returns an empty list when the API responds with no array', async () => {
    requestMock.mockResolvedValueOnce({
      status: 204,
      headers: {},
      data: '',
    })

    const result = await fetchGithubRepositoryContributors({
      token: 'github-token',
      owner: 'acme',
      repo: 'widget',
    })

    expect(result).toEqual({ contributors: [], total_count: 0 })
  })
})
