import type { GithubCacheScope } from './github-cache.js'

export interface GithubCachePolicy {
  operation: string
  scope: GithubCacheScope
  scopeId?: string
  resourceKey: string
  ttlMs: number
  staleMs: number
  tags: string[]
}

export function withGithubPublicScope(policy: GithubCachePolicy): GithubCachePolicy {
  return {
    ...policy,
    scope: 'public',
    scopeId: undefined,
  }
}

const GITHUB_NOTIFICATIONS_CACHE_TTL_MS = 15_000 // 15s
const GITHUB_NOTIFICATIONS_CACHE_STALE_MS = 60_000 // 60s
const GITHUB_USER_REPOSITORIES_CACHE_TTL_MS = 60_000 // 60s
const GITHUB_USER_REPOSITORIES_CACHE_STALE_MS = 5 * 60_000 // 5min
const GITHUB_PULL_REQUEST_SEARCH_CACHE_TTL_MS = 60_000 // 60s
const GITHUB_PULL_REQUEST_SEARCH_CACHE_STALE_MS = 5 * 60_000 // 5min
const GITHUB_PULL_REQUEST_DETAILS_CACHE_TTL_MS = 20_000 // 20s
const GITHUB_PULL_REQUEST_DETAILS_CACHE_STALE_MS = 2 * 60_000 // 2min
const GITHUB_PULL_REQUEST_FILES_CACHE_TTL_MS = 30_000 // 30s
const GITHUB_PULL_REQUEST_FILES_CACHE_STALE_MS = 5 * 60_000 // 5min
const GITHUB_PULL_REQUEST_FILES_BY_COMMIT_CACHE_TTL_MS = 10 * 60_000 // 10min
const GITHUB_PULL_REQUEST_FILES_BY_COMMIT_CACHE_STALE_MS = 60 * 60_000 // 60min
const GITHUB_PULL_REQUEST_COMMITS_CACHE_TTL_MS = 60_000 // 60s
const GITHUB_PULL_REQUEST_COMMITS_CACHE_STALE_MS = 10 * 60_000 // 10min
const GITHUB_PULL_REQUEST_ISSUE_COMMENTS_CACHE_TTL_MS = 15_000 // 15s
const GITHUB_PULL_REQUEST_ISSUE_COMMENTS_CACHE_STALE_MS = 2 * 60_000 // 2min
const GITHUB_PULL_REQUEST_COMMENTS_CACHE_TTL_MS = 15_000 // 15s
const GITHUB_PULL_REQUEST_COMMENTS_CACHE_STALE_MS = 2 * 60_000 // 2min
const GITHUB_PULL_REQUEST_REVIEWS_CACHE_TTL_MS = 20_000 // 20s
const GITHUB_PULL_REQUEST_REVIEWS_CACHE_STALE_MS = 2 * 60_000 // 2min
const GITHUB_REPOSITORY_DETAILS_CACHE_TTL_MS = 2 * 60_000 // 2min
const GITHUB_REPOSITORY_DETAILS_CACHE_STALE_MS = 10 * 60_000 // 10min
const GITHUB_REPOSITORY_README_CACHE_TTL_MS = 2 * 60_000 // 2min
const GITHUB_REPOSITORY_README_CACHE_STALE_MS = 10 * 60_000 // 10min
const GITHUB_REPOSITORY_README_BY_REF_CACHE_TTL_MS = 5 * 60_000 // 5min
const GITHUB_REPOSITORY_README_BY_REF_CACHE_STALE_MS = 30 * 60_000 // 30min
const GITHUB_REPOSITORY_BRANCHES_CACHE_TTL_MS = 60_000 // 60s
const GITHUB_REPOSITORY_BRANCHES_CACHE_STALE_MS = 5 * 60_000 // 5min
const GITHUB_REPOSITORY_PULL_REQUESTS_CACHE_TTL_MS = 30_000 // 30s
const GITHUB_REPOSITORY_PULL_REQUESTS_CACHE_STALE_MS = 5 * 60_000 // 5min
const GITHUB_REPOSITORY_ISSUES_CACHE_TTL_MS = 30_000 // 30s
const GITHUB_REPOSITORY_ISSUES_CACHE_STALE_MS = 5 * 60_000 // 5min
const GITHUB_REPOSITORY_ISSUE_DETAILS_CACHE_TTL_MS = 15_000 // 15s
const GITHUB_REPOSITORY_ISSUE_DETAILS_CACHE_STALE_MS = 2 * 60_000 // 2min
const GITHUB_REPOSITORY_TREE_CACHE_TTL_MS = 10 * 60_000 // 10min
const GITHUB_REPOSITORY_TREE_CACHE_STALE_MS = 24 * 60 * 60_000 // 24h
const GITHUB_REPOSITORY_FILE_CACHE_TTL_MS = 60_000 // 60s
const GITHUB_REPOSITORY_FILE_CACHE_STALE_MS = 10 * 60_000 // 10min
const GITHUB_REPOSITORY_FILE_BY_SHA_CACHE_TTL_MS = 24 * 60 * 60_000 // 24h
const GITHUB_REPOSITORY_FILE_BY_SHA_CACHE_STALE_MS = 7 * 24 * 60 * 60_000 // 7d

function normalizeRepositoryKey(owner: string, repo: string) {
  return `${owner.toLowerCase()}/${repo.toLowerCase()}`
}

function normalizeCacheSegment(value: string) {
  return encodeURIComponent(value.trim())
}

function isGitSha(value: string) {
  return /^[0-9a-f]{40}$/i.test(value.trim())
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

export function getGithubPullRequestFilesTag(owner: string, repo: string, pullNumber: number) {
  return `pull-request:${normalizeRepositoryKey(owner, repo)}:${pullNumber}:files`
}

export function getGithubPullRequestCommitsTag(owner: string, repo: string, pullNumber: number) {
  return `pull-request:${normalizeRepositoryKey(owner, repo)}:${pullNumber}:commits`
}

export function getGithubIssueTag(owner: string, repo: string, issueNumber: number) {
  return `issue:${normalizeRepositoryKey(owner, repo)}:${issueNumber}`
}

export function getGithubIssueCommentsTag(owner: string, repo: string, issueNumber: number) {
  return `issue:${normalizeRepositoryKey(owner, repo)}:${issueNumber}:comments`
}

export function getGithubRepositoryDetailsTag(owner: string, repo: string) {
  return `repo:${normalizeRepositoryKey(owner, repo)}:details`
}

export function getGithubRepositoryReadmeTag(owner: string, repo: string) {
  return `repo:${normalizeRepositoryKey(owner, repo)}:readme`
}

export function getGithubRepositoryBranchesTag(owner: string, repo: string) {
  return `repo:${normalizeRepositoryKey(owner, repo)}:branches`
}

export function getGithubRepositoryTreeTag(owner: string, repo: string, treeSha: string) {
  return `repo:${normalizeRepositoryKey(owner, repo)}:tree:${treeSha}`
}

export function getGithubRepositoryFileTag(owner: string, repo: string, ref: string) {
  const repositoryKey = normalizeRepositoryKey(owner, repo)
  const refPrefix = isGitSha(ref) ? 'blob' : 'ref'
  return `repo:${repositoryKey}:file:${refPrefix}:${normalizeCacheSegment(ref)}`
}

export function createGithubNotificationsCachePolicy(userId: string): GithubCachePolicy {
  return {
    operation: 'viewer.notifications',
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
    operation: 'viewer.repositories',
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
  cacheKey: string,
): GithubCachePolicy {
  return {
    operation: 'viewer.pull_requests.search',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `search:pull-requests:${normalizeCacheSegment(cacheKey)}`,
    ttlMs: GITHUB_PULL_REQUEST_SEARCH_CACHE_TTL_MS,
    staleMs: GITHUB_PULL_REQUEST_SEARCH_CACHE_STALE_MS,
    tags: [getGithubPullRequestSearchTag(userId)],
  }
}

export function createGithubIssueSearchCachePolicy(
  userId: string,
  cacheKey: string,
): GithubCachePolicy {
  return {
    operation: 'viewer.issues.search',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `search:issues:${normalizeCacheSegment(cacheKey)}`,
    ttlMs: GITHUB_PULL_REQUEST_SEARCH_CACHE_TTL_MS,
    staleMs: GITHUB_PULL_REQUEST_SEARCH_CACHE_STALE_MS,
    tags: [`issue-search:${userId}`],
  }
}

export function createGithubPullRequestDetailsCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  pullNumber: number,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'pull_request.details',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `pull-request:${repositoryKey}:${pullNumber}:details`,
    ttlMs: GITHUB_PULL_REQUEST_DETAILS_CACHE_TTL_MS,
    staleMs: GITHUB_PULL_REQUEST_DETAILS_CACHE_STALE_MS,
    tags: [getGithubPullRequestTag(owner, repo, pullNumber)],
  }
}

export function createGithubPullRequestFilesCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  pullNumber: number,
  commitSha?: string,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)
  const scopeSuffix = commitSha
    ? `commit:${normalizeCacheSegment(commitSha)}`
    : 'latest'

  return {
    operation: commitSha ? 'pull_request.files.commit' : 'pull_request.files',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `pull-request:${repositoryKey}:${pullNumber}:files:${scopeSuffix}`,
    ttlMs: commitSha
      ? GITHUB_PULL_REQUEST_FILES_BY_COMMIT_CACHE_TTL_MS
      : GITHUB_PULL_REQUEST_FILES_CACHE_TTL_MS,
    staleMs: commitSha
      ? GITHUB_PULL_REQUEST_FILES_BY_COMMIT_CACHE_STALE_MS
      : GITHUB_PULL_REQUEST_FILES_CACHE_STALE_MS,
    tags: [getGithubPullRequestFilesTag(owner, repo, pullNumber)],
  }
}

export function createGithubPullRequestCommitsCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  pullNumber: number,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'pull_request.commits',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `pull-request:${repositoryKey}:${pullNumber}:commits`,
    ttlMs: GITHUB_PULL_REQUEST_COMMITS_CACHE_TTL_MS,
    staleMs: GITHUB_PULL_REQUEST_COMMITS_CACHE_STALE_MS,
    tags: [getGithubPullRequestCommitsTag(owner, repo, pullNumber)],
  }
}

export function createGithubPullRequestIssueCommentsCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  pullNumber: number,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'pull_request.issue_comments',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `pull-request:${repositoryKey}:${pullNumber}:issue-comments`,
    ttlMs: GITHUB_PULL_REQUEST_ISSUE_COMMENTS_CACHE_TTL_MS,
    staleMs: GITHUB_PULL_REQUEST_ISSUE_COMMENTS_CACHE_STALE_MS,
    tags: [getGithubIssueCommentsTag(owner, repo, pullNumber)],
  }
}

export function createGithubPullRequestCommentsCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  pullNumber: number,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'pull_request.comments',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `pull-request:${repositoryKey}:${pullNumber}:comments`,
    ttlMs: GITHUB_PULL_REQUEST_COMMENTS_CACHE_TTL_MS,
    staleMs: GITHUB_PULL_REQUEST_COMMENTS_CACHE_STALE_MS,
    tags: [getGithubPullRequestCommentsTag(owner, repo, pullNumber)],
  }
}

export function createGithubPullRequestReviewsCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  pullNumber: number,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'pull_request.reviews',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `pull-request:${repositoryKey}:${pullNumber}:reviews`,
    ttlMs: GITHUB_PULL_REQUEST_REVIEWS_CACHE_TTL_MS,
    staleMs: GITHUB_PULL_REQUEST_REVIEWS_CACHE_STALE_MS,
    tags: [getGithubPullRequestReviewsTag(owner, repo, pullNumber)],
  }
}

export function createGithubRepositoryDetailsCachePolicy(
  userId: string,
  owner: string,
  repo: string,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'repository.details',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `repo:${repositoryKey}:details`,
    ttlMs: GITHUB_REPOSITORY_DETAILS_CACHE_TTL_MS,
    staleMs: GITHUB_REPOSITORY_DETAILS_CACHE_STALE_MS,
    tags: [getGithubRepositoryDetailsTag(owner, repo)],
  }
}

export function createGithubRepositoryReadmeCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  ref?: string,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)
  const normalizedRef = ref?.trim()

  return {
    operation: normalizedRef
      ? 'repository.readme.ref'
      : 'repository.readme',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: normalizedRef
      ? `repo:${repositoryKey}:readme:ref:${normalizeCacheSegment(normalizedRef)}`
      : `repo:${repositoryKey}:readme`,
    ttlMs: normalizedRef
      ? GITHUB_REPOSITORY_README_BY_REF_CACHE_TTL_MS
      : GITHUB_REPOSITORY_README_CACHE_TTL_MS,
    staleMs: normalizedRef
      ? GITHUB_REPOSITORY_README_BY_REF_CACHE_STALE_MS
      : GITHUB_REPOSITORY_README_CACHE_STALE_MS,
    tags: [getGithubRepositoryReadmeTag(owner, repo)],
  }
}

export function createGithubRepositoryBranchesCachePolicy(
  userId: string,
  owner: string,
  repo: string,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'repository.branches',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `repo:${repositoryKey}:branches`,
    ttlMs: GITHUB_REPOSITORY_BRANCHES_CACHE_TTL_MS,
    staleMs: GITHUB_REPOSITORY_BRANCHES_CACHE_STALE_MS,
    tags: [getGithubRepositoryBranchesTag(owner, repo)],
  }
}

export function createGithubRepositoryPullRequestsCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  filtersCacheKey?: string,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)
  const filtersKey = filtersCacheKey ? `:${encodeURIComponent(filtersCacheKey)}` : ''

  return {
    operation: 'repository.pull_requests',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `repo:${repositoryKey}:pull-requests${filtersKey}`,
    ttlMs: GITHUB_REPOSITORY_PULL_REQUESTS_CACHE_TTL_MS,
    staleMs: GITHUB_REPOSITORY_PULL_REQUESTS_CACHE_STALE_MS,
    tags: [getGithubRepoPullRequestsTag(owner, repo)],
  }
}

export function createGithubRepositoryIssuesCachePolicy(
  userId: string,
  owner: string,
  repo: string,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'repository.issues',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `repo:${repositoryKey}:issues`,
    ttlMs: GITHUB_REPOSITORY_ISSUES_CACHE_TTL_MS,
    staleMs: GITHUB_REPOSITORY_ISSUES_CACHE_STALE_MS,
    tags: [getGithubRepoIssuesTag(owner, repo)],
  }
}

export function createGithubRepositoryIssueDetailsCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  issueNumber: number,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: 'repository.issue_details',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `repo:${repositoryKey}:issue:${issueNumber}`,
    ttlMs: GITHUB_REPOSITORY_ISSUE_DETAILS_CACHE_TTL_MS,
    staleMs: GITHUB_REPOSITORY_ISSUE_DETAILS_CACHE_STALE_MS,
    tags: [
      getGithubIssueTag(owner, repo, issueNumber),
      getGithubIssueCommentsTag(owner, repo, issueNumber),
    ],
  }
}

export function createGithubRepositoryTreeCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  treeSha: string,
  recursive?: string,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)

  return {
    operation: recursive === undefined
      ? 'repository.tree'
      : 'repository.tree.recursive',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: recursive === undefined
      ? `repo:${repositoryKey}:tree:${normalizeCacheSegment(treeSha)}`
      : `repo:${repositoryKey}:tree:${normalizeCacheSegment(treeSha)}:recursive:${normalizeCacheSegment(recursive)}`,
    ttlMs: GITHUB_REPOSITORY_TREE_CACHE_TTL_MS,
    staleMs: GITHUB_REPOSITORY_TREE_CACHE_STALE_MS,
    tags: [getGithubRepositoryTreeTag(owner, repo, treeSha)],
  }
}

export function createGithubRepositoryFileCachePolicy(
  userId: string,
  owner: string,
  repo: string,
  path: string,
  ref: string,
): GithubCachePolicy {
  const repositoryKey = normalizeRepositoryKey(owner, repo)
  const bySha = isGitSha(ref)
  const normalizedRef = normalizeCacheSegment(ref)
  const normalizedPath = normalizeCacheSegment(path)

  return {
    operation: bySha ? 'repository.file.blob' : 'repository.file',
    scope: 'viewer',
    scopeId: userId,
    resourceKey: `repo:${repositoryKey}:file:${bySha ? 'blob' : 'ref'}:${normalizedRef}:path:${normalizedPath}`,
    ttlMs: bySha
      ? GITHUB_REPOSITORY_FILE_BY_SHA_CACHE_TTL_MS
      : GITHUB_REPOSITORY_FILE_CACHE_TTL_MS,
    staleMs: bySha
      ? GITHUB_REPOSITORY_FILE_BY_SHA_CACHE_STALE_MS
      : GITHUB_REPOSITORY_FILE_CACHE_STALE_MS,
    tags: [getGithubRepositoryFileTag(owner, repo, ref)],
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
