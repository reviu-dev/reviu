import type { GithubCacheScope } from './github-cache.js'

export type GithubPullRequestSearchCacheVariant = 'latest' | 'need-review'

export interface GithubCachePolicy {
  scope: GithubCacheScope
  scopeId?: string
  resourceKey: string
  ttlMs: number
  staleMs: number
  tags: string[]
}

const GITHUB_NOTIFICATIONS_CACHE_TTL_MS = 15_000 // 15s
const GITHUB_NOTIFICATIONS_CACHE_STALE_MS = 60_000 // 60s
const GITHUB_USER_REPOSITORIES_CACHE_TTL_MS = 60_000 // 60s
const GITHUB_USER_REPOSITORIES_CACHE_STALE_MS = 5 * 60_000 // 5min
const GITHUB_PULL_REQUEST_SEARCH_CACHE_TTL_MS = 60_000 // 60s
const GITHUB_PULL_REQUEST_SEARCH_CACHE_STALE_MS = 5 * 60_000 // 5min

function normalizeRepositoryKey(owner: string, repo: string) {
  return `${owner.toLowerCase()}/${repo.toLowerCase()}`
}

function uniq(tags: string[]) {
  return [...new Set(tags)]
}

export function getGithubNotificationsTag(userId: string) {
  return `viewer:${userId}:notifications`
}

export function getGithubUserRepositoriesTag(userId: string) {
  return `viewer:${userId}:repos-me`
}

export function getGithubPullRequestSearchTag(userId: string) {
  return `viewer:${userId}:pr-search`
}

export function getGithubPullRequestSearchVariantTag(
  userId: string,
  variant: GithubPullRequestSearchCacheVariant,
) {
  return `viewer:${userId}:pr-search:${variant}`
}

export function getGithubRepoPullRequestsTag(owner: string, repo: string) {
  return `repo:${normalizeRepositoryKey(owner, repo)}:pull-requests`
}

export function getGithubRepoIssuesTag(owner: string, repo: string) {
  return `repo:${normalizeRepositoryKey(owner, repo)}:issues`
}

export function getGithubPullRequestTag(owner: string, repo: string, pullNumber: number) {
  return `pull-request:${normalizeRepositoryKey(owner, repo)}:${pullNumber}`
}

export function getGithubPullRequestCommentsTag(owner: string, repo: string, pullNumber: number) {
  return `pull-request:${normalizeRepositoryKey(owner, repo)}:${pullNumber}:comments`
}

export function getGithubPullRequestReviewsTag(owner: string, repo: string, pullNumber: number) {
  return `pull-request:${normalizeRepositoryKey(owner, repo)}:${pullNumber}:reviews`
}

export function getGithubIssueTag(owner: string, repo: string, issueNumber: number) {
  return `issue:${normalizeRepositoryKey(owner, repo)}:${issueNumber}`
}

export function getGithubIssueCommentsTag(owner: string, repo: string, issueNumber: number) {
  return `issue:${normalizeRepositoryKey(owner, repo)}:${issueNumber}:comments`
}

export function createGithubNotificationsCachePolicy(userId: string): GithubCachePolicy {
  return {
    scope: 'viewer',
    scopeId: userId,
    resourceKey: 'notifications',
    ttlMs: GITHUB_NOTIFICATIONS_CACHE_TTL_MS,
    staleMs: GITHUB_NOTIFICATIONS_CACHE_STALE_MS,
    tags: [getGithubNotificationsTag(userId)],
  }
}

export function createGithubUserRepositoriesCachePolicy(userId: string): GithubCachePolicy {
  return {
    scope: 'viewer',
    scopeId: userId,
    resourceKey: 'repos:me',
    ttlMs: GITHUB_USER_REPOSITORIES_CACHE_TTL_MS,
    staleMs: GITHUB_USER_REPOSITORIES_CACHE_STALE_MS,
    tags: [getGithubUserRepositoriesTag(userId)],
  }
}

export function createGithubPullRequestSearchCachePolicy(
  userId: string,
  variant: GithubPullRequestSearchCacheVariant,
): GithubCachePolicy {
  const resourceKey = variant === 'latest'
    ? 'search:latest-pull-requests'
    : 'search:need-review-pull-requests'

  return {
    scope: 'viewer',
    scopeId: userId,
    resourceKey,
    ttlMs: GITHUB_PULL_REQUEST_SEARCH_CACHE_TTL_MS,
    staleMs: GITHUB_PULL_REQUEST_SEARCH_CACHE_STALE_MS,
    tags: [
      getGithubPullRequestSearchTag(userId),
      getGithubPullRequestSearchVariantTag(userId, variant),
    ],
  }
}

export function getGithubPullRequestMutationTags(
  {
    userId,
    owner,
    repo,
    pullNumber,
    includeComments = false,
    includeReviews = false,
  }: {
    userId: string
    owner: string
    repo: string
    pullNumber: number
    includeComments?: boolean
    includeReviews?: boolean
  },
) {
  const tags = [
    getGithubPullRequestSearchTag(userId),
    getGithubRepoPullRequestsTag(owner, repo),
    getGithubPullRequestTag(owner, repo, pullNumber),
  ]

  if (includeComments) {
    tags.push(getGithubPullRequestCommentsTag(owner, repo, pullNumber))
  }

  if (includeReviews) {
    tags.push(getGithubPullRequestReviewsTag(owner, repo, pullNumber))
  }

  return uniq(tags)
}

export function getGithubIssueMutationTags(
  {
    owner,
    repo,
    issueNumber,
    includeComments = false,
  }: {
    owner: string
    repo: string
    issueNumber: number
    includeComments?: boolean
  },
) {
  const tags = [
    getGithubRepoIssuesTag(owner, repo),
    getGithubIssueTag(owner, repo, issueNumber),
  ]

  if (includeComments) {
    tags.push(getGithubIssueCommentsTag(owner, repo, issueNumber))
  }

  return uniq(tags)
}
