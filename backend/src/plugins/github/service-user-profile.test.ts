import { afterEach, describe, expect, it, vi } from 'vitest'

import { fetchGithubUserProfileGraphql } from './service.js'

const { requestMock } = vi.hoisted(() => ({
  requestMock: vi.fn(),
}))

vi.mock('@octokit/request', () => ({
  request: requestMock,
}))

describe('github user profile graphql service', () => {
  afterEach(() => {
    requestMock.mockReset()
  })

  it('maps profile fields, repository stats, and aggregated languages', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: {
        data: {
          user: {
            login: 'octocat',
            name: 'The Octocat',
            avatarUrl: 'https://avatars.githubusercontent.com/u/583231?v=4',
            bio: 'GitHub mascot',
            company: '@github',
            location: 'San Francisco',
            websiteUrl: 'https://github.blog',
            twitterUsername: 'octocat',
            url: 'https://github.com/octocat',
            createdAt: '2011-01-25T18:44:36Z',
            followers: { totalCount: 99 },
            following: { totalCount: 5 },
            repositories: {
              totalCount: 3,
              nodes: [
                {
                  name: 'hello-world',
                  nameWithOwner: 'octocat/hello-world',
                  description: 'Example repo',
                  isPrivate: false,
                  isFork: false,
                  isArchived: false,
                  url: 'https://github.com/octocat/hello-world',
                  stargazerCount: 12,
                  forkCount: 3,
                  updatedAt: '2026-04-10T10:00:00Z',
                  pushedAt: '2026-04-10T09:00:00Z',
                  owner: { login: 'octocat' },
                  primaryLanguage: { name: 'TypeScript', color: '#3178c6' },
                  languages: {
                    totalSize: 1000,
                    edges: [
                      { size: 700, node: { name: 'TypeScript', color: '#3178c6' } },
                      { size: 300, node: { name: 'JavaScript', color: '#f1e05a' } },
                    ],
                  },
                },
                {
                  name: 'rusty',
                  nameWithOwner: 'octocat/rusty',
                  description: null,
                  isPrivate: true,
                  isFork: true,
                  isArchived: false,
                  url: 'https://github.com/octocat/rusty',
                  stargazerCount: 8,
                  forkCount: 1,
                  updatedAt: '2026-04-08T10:00:00Z',
                  pushedAt: null,
                  owner: { login: 'octocat' },
                  primaryLanguage: { name: 'Rust', color: '#dea584' },
                  languages: {
                    totalSize: 1000,
                    edges: [
                      { size: 700, node: { name: 'Rust', color: '#dea584' } },
                      { size: 300, node: { name: 'TypeScript', color: '#3178c6' } },
                    ],
                  },
                },
              ],
            },
          },
        },
      },
    })

    const profile = await fetchGithubUserProfileGraphql({
      token: 'github-token',
      login: 'octocat',
      repositoriesLimit: 100,
    })

    expect(requestMock).toHaveBeenCalledWith(
      'POST /graphql',
      expect.objectContaining({
        variables: { login: 'octocat', first: 100 },
      }),
    )
    expect(profile).toMatchObject({
      login: 'octocat',
      name: 'The Octocat',
      repositories_count: 3,
      repositories_indexed_count: 2,
      repositories_truncated: true,
      stargazers_count: 20,
      forks_count: 4,
    })
    expect(profile?.languages).toEqual([
      { name: 'TypeScript', color: '#3178c6', size: 1000, percentage: 50 },
      { name: 'Rust', color: '#dea584', size: 700, percentage: 35 },
      { name: 'JavaScript', color: '#f1e05a', size: 300, percentage: 15 },
    ])
    expect(profile?.repositories[0]).toMatchObject({
      owner: 'octocat',
      repo: 'hello-world',
      full_name: 'octocat/hello-world',
      language: 'TypeScript',
      stargazers_count: 12,
    })
  })

  it('returns null when GitHub has no user for the login', async () => {
    requestMock.mockResolvedValueOnce({
      status: 200,
      headers: {},
      data: { data: { user: null } },
    })

    await expect(fetchGithubUserProfileGraphql({
      token: 'github-token',
      login: 'missing',
      repositoriesLimit: 100,
    })).resolves.toBeNull()
  })
})
