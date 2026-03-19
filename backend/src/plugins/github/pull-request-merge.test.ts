import type {
  GithubRepositoryResponse,
  PullRequestDetailsResponse,
  PullRequestParams,
} from './types.js'
import { describe, expect, it, vi } from 'vitest'
import {
  buildGithubPullRequestMergeReadiness,
  fetchGithubPullRequestMergeReadiness,
} from './pull-request-merge.js'

function makePullRequest(overrides: Record<string, unknown> = {}): PullRequestDetailsResponse {
  return {
    merged: false,
    merged_at: null,
    draft: false,
    state: 'open',
    mergeable: true,
    mergeable_state: 'clean',
    rebaseable: true,
    auto_merge: null,
    head: {
      sha: 'head-sha',
    },
    base: {
      repo: {
        allow_merge_commit: true,
        allow_squash_merge: true,
        allow_rebase_merge: true,
        permissions: {
          admin: true,
          maintain: true,
          push: true,
          triage: true,
          pull: true,
        },
      },
    },
    ...overrides,
  } as unknown as PullRequestDetailsResponse
}

function makeParams(): PullRequestParams {
  return {
    owner: 'acme',
    repo: 'widget',
    pull_number: 42,
  }
}

function makeRepository(overrides: Record<string, unknown> = {}): GithubRepositoryResponse {
  return {
    allow_merge_commit: true,
    allow_squash_merge: true,
    allow_rebase_merge: true,
    permissions: {
      admin: true,
      maintain: true,
      push: true,
      triage: true,
      pull: true,
    },
    ...overrides,
  } as unknown as GithubRepositoryResponse
}

describe('pull request merge readiness', () => {
  it('maps a mergeable pull request with repository permissions to ready', () => {
    expect(buildGithubPullRequestMergeReadiness(makePullRequest())).toEqual({
      status: 'ready',
      message: 'This pull request is ready to merge.',
      current_head_sha: 'head-sha',
      available_methods: ['merge', 'squash', 'rebase'],
      default_method: 'merge',
      can_merge_now: true,
      viewer_can_merge: true,
      mergeable_state: 'clean',
      rebaseable: true,
      auto_merge_enabled: false,
    })
  })

  it('maps missing permissions to forbidden before mergeability', () => {
    const readiness = buildGithubPullRequestMergeReadiness(makePullRequest({
      base: {
        repo: {
          allow_merge_commit: true,
          allow_squash_merge: true,
          allow_rebase_merge: true,
          permissions: {
            admin: false,
            maintain: false,
            push: false,
            triage: true,
            pull: true,
          },
        },
      },
    }))

    expect(readiness.status).toBe('forbidden')
    expect(readiness.can_merge_now).toBe(false)
    expect(readiness.viewer_can_merge).toBe(false)
  })

  it('uses repository fallback metadata when pull request repo fields are missing', () => {
    const readiness = buildGithubPullRequestMergeReadiness(
      makePullRequest({
        base: {
          repo: {
            permissions: null,
            allow_merge_commit: undefined,
            allow_squash_merge: undefined,
            allow_rebase_merge: undefined,
          },
        },
      }),
      makeRepository(),
    )

    expect(readiness.status).toBe('ready')
    expect(readiness.viewer_can_merge).toBe(true)
    expect(readiness.available_methods).toEqual(['merge', 'squash', 'rebase'])
  })

  it('maps draft, merged, and closed pull requests to non-mergeable states', () => {
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({ draft: true })).status).toBe('draft')
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      merged: true,
      merged_at: '2026-03-19T10:10:00Z',
    })).status).toBe('merged')
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({ state: 'closed' })).status).toBe('closed')
  })

  it('maps blocked mergeable states to dedicated messages', () => {
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      mergeable: false,
      mergeable_state: 'dirty',
    })).message).toContain('merge conflicts')
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      mergeable: false,
      mergeable_state: 'behind',
    })).message).toContain('out of date')
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      mergeable: false,
      mergeable_state: 'blocked',
    })).message).toContain('required reviews')
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      mergeable: false,
      mergeable_state: 'unstable',
    })).message).toContain('finalizing merge checks')
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      mergeable: false,
      mergeable_state: 'mystery',
    })).message).toContain('cannot be merged right now')
  })

  it('keeps GitHub blocked states blocked even when mergeable is true', () => {
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      mergeable: true,
      mergeable_state: 'blocked',
    })).status).toBe('blocked')
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      mergeable: true,
      mergeable_state: 'unstable',
    })).status).toBe('blocked')
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      mergeable: true,
      mergeable_state: 'has_hooks',
    })).status).toBe('ready')
  })

  it('filters rebase when GitHub reports the pull request is not rebaseable', () => {
    expect(buildGithubPullRequestMergeReadiness(makePullRequest({
      rebaseable: false,
    })).available_methods).toEqual(['merge', 'squash'])
  })

  it('retries boundedly while GitHub keeps mergeability pending', async () => {
    const fetchPullRequest = vi.fn()
      .mockResolvedValueOnce(makePullRequest({ mergeable: null }))
      .mockResolvedValueOnce(makePullRequest({ mergeable: null }))
      .mockResolvedValueOnce(makePullRequest({ mergeable: true }))

    const readiness = await fetchGithubPullRequestMergeReadiness({
      token: 'token',
      params: makeParams(),
      fetchPullRequest,
    })

    expect(fetchPullRequest).toHaveBeenCalledTimes(3)
    expect(readiness.status).toBe('ready')
  })

  it('fetches repository metadata when the pull request payload omits permissions and merge settings', async () => {
    const fetchPullRequest = vi.fn().mockResolvedValue(makePullRequest({
      base: {
        repo: {
          permissions: null,
          allow_merge_commit: undefined,
          allow_squash_merge: undefined,
          allow_rebase_merge: undefined,
        },
      },
    }))
    const fetchRepository = vi.fn().mockResolvedValue(makeRepository())

    const readiness = await fetchGithubPullRequestMergeReadiness({
      token: 'token',
      params: makeParams(),
      fetchPullRequest,
      fetchRepository,
    })

    expect(fetchPullRequest).toHaveBeenCalledTimes(1)
    expect(fetchRepository).toHaveBeenCalledTimes(1)
    expect(fetchRepository).toHaveBeenCalledWith({
      token: 'token',
      params: {
        owner: 'acme',
        repo: 'widget',
      },
    })
    expect(readiness.status).toBe('ready')
    expect(readiness.available_methods).toEqual(['merge', 'squash', 'rebase'])
  })

  it('returns checking after exhausting mergeability retries', async () => {
    const fetchPullRequest = vi.fn().mockResolvedValue(makePullRequest({ mergeable: null }))

    const readiness = await fetchGithubPullRequestMergeReadiness({
      token: 'token',
      params: makeParams(),
      fetchPullRequest,
    })

    expect(fetchPullRequest).toHaveBeenCalledTimes(3)
    expect(readiness.status).toBe('checking')
    expect(readiness.can_merge_now).toBe(false)
  })
})
