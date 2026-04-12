import type { Context } from 'hono'
import type { GithubCachePolicy } from '../plugins/github/cache/github-cache-policy.js'
import type {
  GithubCacheLoadedPayload,
  GithubCacheLoadResult,
  GithubCacheNotModifiedPayload,
  GithubCachePaginationMetadata,
  GithubCacheValidator,
  GithubCacheValidators,
} from '../plugins/github/cache/github-cache.js'
import type {
  AddIssueAssigneesParams,
  AddIssueLabelsParams,
  CompareParams,
  CreateIssueCommentParams,
  CreatePullRequestCommentParams,
  CreatePullRequestCommentReplyParams,
  CreatePullRequestParams,
  CreatePullRequestReviewParams,
  DeleteIssueCommentParams,
  DeletePullRequestCommentParams,
  GetContentParams,
  GithubFileAsset,
  GithubFileContent,
  GithubIssue,
  GithubIssueDetails,
  GithubIssueDetailsCommentParameters,
  GithubIssueDetailsParameters,
  GithubIssueParameters,
  GithubNotification,
  GithubPullRequest,
  GithubPullRequestAuthor,
  GithubPullRequestChecksSummary,
  GithubPullRequestCommit,
  GithubPullRequestDetails,
  GithubPullRequestFile,
  GithubPullRequestFilterOptions,
  GithubPullRequestIssueComment,
  GithubPullRequestMergeReadiness,
  GithubPullRequestMergeResult,
  GithubPullRequestReview,
  GithubPullRequestReviewComment,
  GithubPullRequestSearchFilters,
  GithubRepositoryBranch,
  GithubRepositoryBranchesParameters,
  GithubRepositoryDetails,
  GithubRepositoryParameters,
  GithubRepositoryReadme,
  GithubRepositoryReadmeParameters,
  GithubRepositoryTree,
  GithubRepositoryTreeParams,
  GithubUserRepository,
  ListPullsParams,
  MergePullRequestParams,
  NotificationsParams,
  PullRequestCommentsParams,
  PullRequestParams,
  PullRequestReviewsParams,
  RemoveIssueAssigneesParams,
  RemovePullRequestReviewersParams,
  RequestPullRequestReviewersParams,
  UpdateIssueCommentParams,
  UpdateIssueParams,
  UpdatePullRequestBranchParams,
  UpdatePullRequestCommentParams,
  UpdatePullRequestParams,
  UserRepositoriesParams,
} from '../plugins/github/types.js'
import { Buffer } from 'node:buffer'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { logger } from '../lib/logger.js'
import { authMiddlewarePro } from '../middlewares/auth.js'
import {
  createGithubNotificationsCachePolicy,
  createGithubPullRequestCommentsCachePolicy,
  createGithubPullRequestCommitsCachePolicy,
  createGithubPullRequestDetailsCachePolicy,
  createGithubPullRequestFilesCachePolicy,
  createGithubPullRequestIssueCommentsCachePolicy,
  createGithubPullRequestReviewsCachePolicy,
  createGithubPullRequestSearchCachePolicy,
  createGithubRepositoryBranchesCachePolicy,
  createGithubRepositoryDetailsCachePolicy,
  createGithubRepositoryFileCachePolicy,
  createGithubRepositoryIssueDetailsCachePolicy,
  createGithubRepositoryIssuesCachePolicy,
  createGithubRepositoryPullRequestsCachePolicy,
  createGithubRepositoryReadmeCachePolicy,
  createGithubRepositoryTreeCachePolicy,
  createGithubUserRepositoriesCachePolicy,
  getGithubIssueMutationTags,
  getGithubNotificationsTag,
  getGithubPullRequestCommitsTag,
  getGithubPullRequestFilesTag,
  getGithubPullRequestMutationTags,

  withGithubPublicScope,
} from '../plugins/github/cache/github-cache-policy.js'
import { githubCache } from '../plugins/github/cache/github-cache-runtime.js'
import { githubRepositoryVisibility } from '../plugins/github/cache/github-repository-visibility-runtime.js'
import {
  formatGithubUser,
  mapGithubGraphqlPullRequest,
  mapGithubIssueComment,
  mapGithubIssueDescriptionUpdate,
  mapGithubPullRequest,
  mapGithubPullRequestAuthor,
  mapGithubPullRequestCommit,
  mapGithubPullRequestDescriptionUpdate,
  mapGithubPullRequestFile,
  mapGithubPullRequestIssueComment,
  mapGithubPullRequestReview,
  mapGithubPullRequestReviewComment,
} from '../plugins/github/formatter.js'
import {
  buildGithubIssueDetailsValidators,
  GITHUB_ISSUE_DETAILS_COMMENTS_VALIDATOR_KEY,
  GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY,
  mergeGithubIssueDetailsPayload,
} from '../plugins/github/issue-details.js'
import { runWithGithubMetricsContext } from '../plugins/github/metrics/github-metrics-context.js'
import { fetchGithubPullRequestChecksSummary } from '../plugins/github/pull-request-checks.js'
import { fetchGithubPullRequestMergeReadiness } from '../plugins/github/pull-request-merge.js'
import {
  createPullRequestBodySchema,
  createPullRequestLineCommentBodySchema,
  createPullRequestReviewBodySchema,
  createPullRequestThreadReplyBodySchema,
  issueCommentBodySchema,
  mergePullRequestBodySchema,
  pullRequestFilterOptionsBodySchema,
  pullRequestLabelsMutationBodySchema,
  pullRequestSearchBodySchema,
  pullRequestSearchFiltersSchema,
  pullRequestStatusMutationBodySchema,
  pullRequestUsersMutationBodySchema,
  updateDescriptionBodySchema,
  updatePullRequestCommentBodySchema,
} from '../plugins/github/schemas.js'
import {
  addGithubIssueAssignees,
  addGithubIssueLabels,
  compareGithubRefs,
  convertGithubPullRequestToDraft,
  createGithubIssueComment,
  createGithubPullRequest,
  createGithubPullRequestComment,
  createGithubPullRequestCommentReply,
  createGithubPullRequestReview,
  deleteGithubIssueComment,
  deleteGithubPullRequestComment,
  fetchGithubCommitConditionally,
  fetchGithubCommitFilesAllPages,
  fetchGithubNotifications,
  fetchGithubPullRequest,
  fetchGithubPullRequestCommentsAllPages,
  fetchGithubPullRequestCommentsConditionally,
  fetchGithubPullRequestCommitsAllPages,
  fetchGithubPullRequestConditionally,
  fetchGithubPullRequestFilesAllPages,
  fetchGithubPullRequestReviewsConditionally,
  fetchGithubPullRequests,
  fetchGithubPullRequestSearchGraphql,
  fetchGithubRepositoryAssignees,
  fetchGithubRepositoryBranchesConditionally,
  fetchGithubRepositoryConditionally,
  fetchGithubRepositoryContentConditionally,
  fetchGithubRepositoryContentObjectConditionally,
  fetchGithubRepositoryIssueCommentsAllPages,
  fetchGithubRepositoryIssueCommentsConditionally,
  fetchGithubRepositoryIssueConditionally,
  fetchGithubRepositoryIssuesConditionally,
  fetchGithubRepositoryLabels,
  fetchGithubRepositoryReadmeConditionally,
  fetchGithubRepositoryTreesConditionally,
  fetchGithubUserRepositories,
  markGithubNotificationDone,
  markGithubNotificationRead,
  markGithubPullRequestReadyForReview,
  mergeGithubPullRequest,
  patchGithubIssue,
  patchGithubIssueComment,
  patchGithubPullRequest,
  patchGithubPullRequestComment,
  removeGithubIssueAssignees,
  removeGithubIssueLabel,
  removeGithubPullRequestReviewers,
  requestGithubPullRequestReviewers,
  updateGithubPullRequestBranch,
} from '../plugins/github/service.js'

const LATEST_PULL_REQUESTS_LIMIT = 20
const REPOSITORY_PULL_REQUESTS_LIMIT = 100
const GITHUB_PULL_REQUEST_COLLECTION_VALIDATOR_KEY = 'pullRequest'
const GITHUB_PULL_REQUEST_FILES_COMMIT_VALIDATOR_KEY = 'commit'
const GITHUB_UPDATE_BRANCH_POLL_ATTEMPTS = 10
const GITHUB_UPDATE_BRANCH_POLL_INTERVAL_MS = 750

function withGithubMetrics<T>(
  userId: string,
  operation: string,
  callback: () => Promise<T>,
) {
  return runWithGithubMetricsContext({ userId, operation }, callback)
}

function setGithubCacheHeaders(
  ctx: Context,
  result: Pick<GithubCacheLoadResult<unknown>, 'cacheStatus' | 'scope'>,
) {
  ctx.header('x-reviu-cache', result.cacheStatus)
  ctx.header('x-reviu-cache-scope', result.scope)
}

function getCachedValidator(
  cachedEntry: { etag?: string, lastModified?: string, validators?: Record<string, { etag?: string, lastModified?: string }> } | null,
  key: string,
) {
  return cachedEntry?.validators?.[key] ?? {
    etag: cachedEntry?.etag,
    lastModified: cachedEntry?.lastModified,
  }
}

function getCachedPaginationMetadata(
  cachedEntry: { pagination?: GithubCachePaginationMetadata } | null,
): GithubCachePaginationMetadata | null {
  return cachedEntry?.pagination ?? null
}

function canConditionallyRevalidateSinglePageCollection(
  pagination: GithubCachePaginationMetadata | null,
) {
  return Boolean(pagination && !pagination.truncated && pagination.pageCount <= 1)
}

function buildPaginationMetadata(
  pageCount: number,
  itemCount: number,
  truncated: boolean,
): GithubCachePaginationMetadata {
  return {
    pageCount,
    itemCount,
    truncated,
  }
}

function buildNamedValidator(
  key: string,
  validator: GithubCacheValidator,
): GithubCacheValidators {
  return {
    [key]: validator,
  }
}

function buildNotModifiedCacheResult(
  key: string,
  validator: GithubCacheValidator,
): GithubCacheNotModifiedPayload {
  return {
    notModified: true,
    etag: validator.etag,
    lastModified: validator.lastModified,
    validators: buildNamedValidator(key, validator),
  }
}

function buildLoadedCacheResult<T>(
  key: string,
  validator: GithubCacheValidator,
  payload: T,
): GithubCacheLoadedPayload<T> {
  return {
    payload,
    etag: validator.etag,
    lastModified: validator.lastModified,
    validators: buildNamedValidator(key, validator),
  }
}

async function resolveRepositoryReadCachePolicy(
  cachePolicy: GithubCachePolicy,
  owner: string,
  repo: string,
) {
  const isKnownPublic = await githubRepositoryVisibility.isKnownPublic(owner, repo)
  return isKnownPublic ? withGithubPublicScope(cachePolicy) : cachePolicy
}

async function syncRepositoryPublicVisibility(
  owner: string,
  repo: string,
  isPrivate: boolean,
) {
  try {
    if (isPrivate) {
      await githubRepositoryVisibility.clear(owner, repo)
      return
    }

    await githubRepositoryVisibility.markPublic(owner, repo)
  }
  catch (error) {
    logger.warn({ error, owner, repo }, 'Failed to sync GitHub repository public visibility')
  }
}

async function syncUserRepositoriesPublicVisibility(repositories: GithubUserRepository[]) {
  await Promise.allSettled(
    repositories.map(repository => syncRepositoryPublicVisibility(
      repository.owner,
      repository.repo,
      repository.private,
    )),
  )
}

function normalizeSearchValue(value: string) {
  return value.trim()
}

function quoteGithubSearchValue(value: string) {
  return `"${value.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"`
}

function buildGithubSearchQualifier(
  qualifier: string,
  value: string,
  { quote = false }: { quote?: boolean } = {},
) {
  const trimmed = normalizeSearchValue(value)
  if (!trimmed) {
    return null
  }

  return `${qualifier}:${quote ? quoteGithubSearchValue(trimmed) : trimmed}`
}

function buildGithubSearchAnyOfGroup(qualifiers: Array<string | null>) {
  const values = qualifiers.filter((value): value is string => Boolean(value))
  if (values.length === 0) {
    return null
  }
  if (values.length === 1) {
    return values[0]
  }
  return `(${values.join(' OR ')})`
}

function buildRequestedReviewerQualifier(login: string) {
  const trimmed = normalizeSearchValue(login)
  if (!trimmed) {
    return null
  }

  if (trimmed === '@me') {
    return 'user-review-requested:@me'
  }

  return buildGithubSearchQualifier('review-requested', trimmed)
}

function normalizeRepositoryFilter(repo: string) {
  const trimmed = repo.trim()
  if (!trimmed) {
    return null
  }

  const [owner, name] = trimmed.split('/')
  if (!owner || !name) {
    return null
  }

  return `${owner.trim()}/${name.trim()}`
}

function pullRequestSearchSortQualifier(sort: GithubPullRequestSearchFilters['sort']) {
  switch (sort) {
    case 'created_desc':
      return 'created-desc'
    case 'created_asc':
      return 'created-asc'
    case 'comments_desc':
      return 'comments-desc'
    case 'updated_desc':
    default:
      return 'updated-desc'
  }
}

function buildPullRequestSearchQuery(
  filters: GithubPullRequestSearchFilters,
  options: { openOnly?: boolean } = {},
) {
  const parts = ['is:pr']

  if (options.openOnly ?? true) {
    parts.push('state:open')
  }
  parts.push('archived:false')

  const repoQualifiers = filters.repos
    .map(repo => buildGithubSearchQualifier('repo', normalizeRepositoryFilter(repo) ?? ''))
    .filter((value): value is string => Boolean(value))
  if (repoQualifiers.length > 0) {
    parts.push(...repoQualifiers)
  }

  const labelGroup = buildGithubSearchAnyOfGroup(
    filters.labels.map(label => buildGithubSearchQualifier('label', label, { quote: true })),
  )
  if (labelGroup) {
    parts.push(labelGroup)
  }

  const authorGroup = buildGithubSearchAnyOfGroup(
    filters.authors.map(author => buildGithubSearchQualifier('author', author)),
  )
  if (authorGroup) {
    parts.push(authorGroup)
  }

  const assigneeGroup = buildGithubSearchAnyOfGroup(
    filters.assignees.map(assignee => buildGithubSearchQualifier('assignee', assignee)),
  )
  if (assigneeGroup) {
    parts.push(assigneeGroup)
  }

  const requestedReviewerGroup = buildGithubSearchAnyOfGroup(
    filters.requested_reviewers.map(login => buildRequestedReviewerQualifier(login)),
  )
  if (requestedReviewerGroup) {
    parts.push(requestedReviewerGroup)
  }

  if (filters.review_status !== 'any') {
    parts.push(`review:${filters.review_status}`)
  }

  if (!filters.include_drafts) {
    parts.push('draft:false')
  }

  const base = filters.base?.trim()
  if (base) {
    const baseQualifier = buildGithubSearchQualifier('base', base)
    if (baseQualifier) {
      parts.push(baseQualifier)
    }
  }

  parts.push(`sort:${pullRequestSearchSortQualifier(filters.sort)}`)

  return parts.join(' ')
}

function normalizePullRequestSearchCacheKey(filters: GithubPullRequestSearchFilters) {
  return JSON.stringify({
    repos: [...filters.repos].sort(),
    labels: [...filters.labels].sort(),
    authors: [...filters.authors].sort(),
    assignees: [...filters.assignees].sort(),
    requested_reviewers: [...filters.requested_reviewers].sort(),
    review_status: filters.review_status,
    include_drafts: filters.include_drafts,
    base: filters.base,
    sort: filters.sort,
  })
}

function uniqueSearchParamValues(params: URLSearchParams, name: string) {
  const seen = new Set<string>()
  return params.getAll(name)
    .flatMap(value => value.split(','))
    .map(value => value.trim())
    .filter((value) => {
      const key = value.toLowerCase()
      if (!key || seen.has(key)) {
        return false
      }
      seen.add(key)
      return true
    })
}

function parseBooleanSearchParam(params: URLSearchParams, name: string, defaultValue: boolean) {
  const value = params.get(name)?.trim().toLowerCase()
  if (!value) {
    return defaultValue
  }
  return value !== 'false'
}

function repositoryPullRequestFiltersInput(url: string) {
  const params = new URL(url).searchParams

  return {
    repos: [],
    labels: uniqueSearchParamValues(params, 'label'),
    authors: uniqueSearchParamValues(params, 'author'),
    assignees: uniqueSearchParamValues(params, 'assignee'),
    requested_reviewers: uniqueSearchParamValues(params, 'requested_reviewer'),
    review_status: params.get('review_status') ?? undefined,
    include_drafts: parseBooleanSearchParam(params, 'include_drafts', true),
    base: params.get('base') ?? undefined,
    sort: params.get('sort') ?? undefined,
  }
}

function makeFilterOptionUser(
  login: string,
  avatarUrl: string | null | undefined,
) {
  return {
    login,
    avatar_url: avatarUrl ?? null,
  }
}

function dedupeFilterOptionUsers(
  users: Array<{ login: string, avatar_url: string | null }>,
) {
  const seen = new Set<string>()
  return users.filter((user) => {
    const key = user.login.trim().toLowerCase()
    if (!key || seen.has(key)) {
      return false
    }
    seen.add(key)
    return true
  })
}

async function fetchPullRequestsSearchWithCache(
  userId: string,
  githubToken: string,
  filters: GithubPullRequestSearchFilters,
) {
  const query = buildPullRequestSearchQuery(filters)
  const cachePolicy = createGithubPullRequestSearchCachePolicy(
    userId,
    normalizePullRequestSearchCacheKey(filters),
  )

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad({
      ...cachePolicy,
      load: async () => {
        const nodes = await fetchGithubPullRequestSearchGraphql({
          token: githubToken,
          query,
          limit: LATEST_PULL_REQUESTS_LIMIT,
        })

        return {
          payload: nodes.map(node => mapGithubGraphqlPullRequest(node).pullRequest),
        }
      },
    }))
}

async function fetchPullRequestFilterOptions(
  githubToken: string,
  repos: string[],
): Promise<GithubPullRequestFilterOptions> {
  const normalizedRepos = [...new Set(repos.map(repo => normalizeRepositoryFilter(repo)).filter(Boolean))] as string[]

  if (normalizedRepos.length === 0) {
    return {
      labels: [],
      authors: [],
      assignees: [],
    }
  }

  const repositoryEntries = normalizedRepos.map((fullName) => {
    const [owner, repo] = fullName.split('/')
    return { owner, repo }
  })

  const [labelsResults, assigneesResults, authorNodes] = await Promise.all([
    Promise.allSettled(
      repositoryEntries.map(({ owner, repo }) =>
        fetchGithubRepositoryLabels({
          token: githubToken,
          params: {
            owner,
            repo,
            per_page: 100,
          },
        })),
    ),
    Promise.allSettled(
      repositoryEntries.map(({ owner, repo }) =>
        fetchGithubRepositoryAssignees({
          token: githubToken,
          params: {
            owner,
            repo,
            per_page: 100,
          },
        })),
    ),
    fetchGithubPullRequestSearchGraphql({
      token: githubToken,
      query: buildPullRequestSearchQuery({
        repos: normalizedRepos,
        labels: [],
        authors: [],
        assignees: [],
        requested_reviewers: [],
        review_status: 'any',
        include_drafts: true,
        base: null,
        sort: 'updated_desc',
      }),
      limit: 50,
    }).catch(() => []),
  ])

  const labels = labelsResults
    .flatMap((result) => {
      if (result.status !== 'fulfilled') {
        return []
      }
      return result.value
        .flatMap(label => (typeof label.name === 'string' && label.name.trim() ? [{ name: label.name.trim() }] : []))
    })
    .filter((label, index, array) =>
      array.findIndex(candidate => candidate.name.toLowerCase() === label.name.toLowerCase()) === index)
    .sort((a, b) => a.name.localeCompare(b.name))

  const assignees = dedupeFilterOptionUsers(
    assigneesResults.flatMap((result) => {
      if (result.status !== 'fulfilled') {
        return []
      }
      return result.value.flatMap(user =>
        typeof user.login === 'string' && user.login.trim()
          ? [makeFilterOptionUser(user.login.trim(), user.avatar_url)]
          : [])
    }),
  ).sort((a, b) => a.login.localeCompare(b.login))

  const authors = dedupeFilterOptionUsers(
    authorNodes.flatMap(node =>
      typeof node.author?.login === 'string' && node.author.login.trim()
        ? [makeFilterOptionUser(node.author.login.trim(), node.author.avatarUrl ?? null)]
        : []),
  ).sort((a, b) => a.login.localeCompare(b.login))

  return {
    labels,
    authors,
    assignees,
  }
}

async function fetchNotificationsWithCache(
  userId: string,
  githubToken: string,
) {
  const cachePolicy = createGithubNotificationsCachePolicy(userId)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad({
      ...cachePolicy,
      load: async () => {
        const params: NotificationsParams = {
          per_page: 50,
          all: true,
        }

        const data = await fetchGithubNotifications({ token: githubToken, params })

        return {
          payload: data.map(notification => ({
            id: notification.id,
            repository: {
              name: notification.repository.name,
              full_name: notification.repository.full_name,
              owner: {
                login: notification.repository.owner.login,
                avatar_url: notification.repository.owner.avatar_url,
              },
            },
            subject: {
              title: notification.subject.title,
              type: notification.subject.type,
              url: notification.subject.url,
              latest_comment_url: notification.subject.latest_comment_url,
            },
            reason: notification.reason,
            unread: notification.unread,
            updated_at: notification.updated_at,
            last_read_at: notification.last_read_at,
            url: notification.url,
            subscription_url: notification.subscription_url,
          } satisfies GithubNotification)),
        }
      },
    }))
}

async function fetchUserRepositoriesWithCache(
  userId: string,
  githubToken: string,
) {
  const cachePolicy = createGithubUserRepositoriesCachePolicy(userId)

  const result = await withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad({
      ...cachePolicy,
      load: async () => {
        const params: UserRepositoriesParams = {
          sort: 'updated',
          direction: 'desc',
          per_page: 100,
        }

        const data = await fetchGithubUserRepositories({ token: githubToken, params })

        return {
          payload: data
            .map(repo => ({
              owner: repo.owner.login,
              repo: repo.name,
              full_name: repo.full_name,
              description: repo.description,
              private: repo.private,
              owner_avatar_url: repo.owner.avatar_url,
              updated_at: repo.updated_at ?? '',
            } satisfies GithubUserRepository))
            .sort((a, b) => b.updated_at.localeCompare(a.updated_at)),
        }
      },
    }))

  await syncUserRepositoriesPublicVisibility(result.payload)

  return result
}

async function fetchPullRequestDetailsWithCache(
  userId: string,
  githubToken: string,
  org: string,
  repo: string,
  pullNumber: number,
) {
  const baseCachePolicy = createGithubPullRequestDetailsCachePolicy(userId, org, repo, pullNumber)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, org, repo)
  let resolvedRepositoryPrivate: boolean | null = null

  const result = await withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubPullRequestDetails>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: PullRequestParams = {
          owner: org,
          repo,
          pull_number: pullNumber,
        }

        const response = await fetchGithubPullRequestConditionally({
          token: githubToken,
          params,
          etag: cachedEntry?.etag,
          lastModified: cachedEntry?.lastModified,
        })

        if (response.notModified) {
          return {
            notModified: true as const,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }

        const data = response.data!
        resolvedRepositoryPrivate = data.base.repo.private
        const author: GithubPullRequestAuthor = mapGithubPullRequestAuthor(data.user)
        const assignees = dedupeFilterOptionUsers(
          (data.assignees ?? []).flatMap(user =>
            typeof user.login === 'string' && user.login.trim()
              ? [makeFilterOptionUser(user.login.trim(), user.avatar_url)]
              : []),
        )
        const requestedReviewers = dedupeFilterOptionUsers(
          (data.requested_reviewers ?? []).flatMap(user =>
            typeof user.login === 'string' && user.login.trim()
              ? [makeFilterOptionUser(user.login.trim(), user.avatar_url)]
              : []),
        )

        let mergeBaseSha = data.base.sha
        const baseRef = data.base.ref
        const headRef = data.head.ref
        const headOwner = data.head.repo.owner.login

        try {
          const compareParams: CompareParams = {
            owner: org,
            repo,
            basehead: `${baseRef}...${headOwner}:${headRef}`,
          }

          const compare = await compareGithubRefs({ token: githubToken, params: compareParams })

          mergeBaseSha = compare.merge_base_commit.sha
        }
        catch {
          mergeBaseSha = data.base.sha
        }

        return {
          payload: {
            node_id: data.node_id,
            number: data.number,
            title: data.title,
            state: data.state,
            draft: Boolean(data.draft),
            created_at: data.created_at,
            updated_at: data.updated_at,
            merged_at: data.merged_at,
            merge_base_sha: mergeBaseSha,
            base_sha: data.base.sha,
            head_sha: data.head.sha,
            base_ref_name: data.base.ref,
            head_ref_name: data.head.ref,
            body: data.body,
            author,
            assignees,
            requested_reviewers: requestedReviewers,
            comments: data.comments,
            review_comments: data.review_comments,
            commits: data.commits,
            additions: data.additions,
            deletions: data.deletions,
            changed_files: data.changed_files,
            labels: data.labels,
            repository: {
              owner: org,
              repo,
            },
            head_repository: {
              owner: data.head.repo.owner.login,
              repo: data.head.repo.name,
            },
          } satisfies GithubPullRequestDetails,
          etag: response.etag,
          lastModified: response.lastModified,
        }
      },
    }))

  if (resolvedRepositoryPrivate != null) {
    await syncRepositoryPublicVisibility(org, repo, resolvedRepositoryPrivate)
  }

  if (resolvedRepositoryPrivate === false && cachePolicy.scope !== 'public' && result.cacheStatus !== 'stale') {
    await githubCache.prime({
      ...withGithubPublicScope(baseCachePolicy),
      payload: result.payload,
    })
  }

  return result
}

async function fetchPullRequestFilesWithCache(
  userId: string,
  githubToken: string,
  org: string,
  repo: string,
  pullNumber: number,
  commitSha?: string,
) {
  const baseCachePolicy = createGithubPullRequestFilesCachePolicy(userId, org, repo, pullNumber, commitSha)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, org, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubPullRequestFile[]>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        if (commitSha) {
          const cachedCommitValidator = getCachedValidator(cachedEntry, GITHUB_PULL_REQUEST_FILES_COMMIT_VALIDATOR_KEY)
          const commitResponse = await fetchGithubCommitConditionally({
            token: githubToken,
            params: {
              owner: org,
              repo,
              ref: commitSha,
            },
            etag: cachedCommitValidator.etag,
            lastModified: cachedCommitValidator.lastModified,
          })

          const nextCommitValidator = {
            etag: commitResponse.etag ?? cachedCommitValidator.etag,
            lastModified: commitResponse.lastModified ?? cachedCommitValidator.lastModified,
          } satisfies GithubCacheValidator

          if (commitResponse.notModified) {
            return buildNotModifiedCacheResult(
              GITHUB_PULL_REQUEST_FILES_COMMIT_VALIDATOR_KEY,
              nextCommitValidator,
            )
          }

          const files = await fetchGithubCommitFilesAllPages({
            token: githubToken,
            params: {
              owner: org,
              repo,
              ref: commitSha,
            },
          })

          return buildLoadedCacheResult(
            GITHUB_PULL_REQUEST_FILES_COMMIT_VALIDATOR_KEY,
            nextCommitValidator,
            files.map(mapGithubPullRequestFile),
          )
        }

        const cachedPullRequestValidator = getCachedValidator(cachedEntry, GITHUB_PULL_REQUEST_COLLECTION_VALIDATOR_KEY)
        const pullRequestResponse = await fetchGithubPullRequestConditionally({
          token: githubToken,
          params: {
            owner: org,
            repo,
            pull_number: pullNumber,
          },
          etag: cachedPullRequestValidator.etag,
          lastModified: cachedPullRequestValidator.lastModified,
        })

        const nextPullRequestValidator = {
          etag: pullRequestResponse.etag ?? cachedPullRequestValidator.etag,
          lastModified: pullRequestResponse.lastModified ?? cachedPullRequestValidator.lastModified,
        } satisfies GithubCacheValidator

        if (pullRequestResponse.notModified) {
          return buildNotModifiedCacheResult(
            GITHUB_PULL_REQUEST_COLLECTION_VALIDATOR_KEY,
            nextPullRequestValidator,
          )
        }

        const files = await fetchGithubPullRequestFilesAllPages({
          token: githubToken,
          params: {
            owner: org,
            repo,
            pull_number: pullNumber,
          },
        })

        return buildLoadedCacheResult(
          GITHUB_PULL_REQUEST_COLLECTION_VALIDATOR_KEY,
          nextPullRequestValidator,
          files.map(mapGithubPullRequestFile),
        )
      },
    }))
}

async function fetchPullRequestCommitsWithCache(
  userId: string,
  githubToken: string,
  org: string,
  repo: string,
  pullNumber: number,
) {
  const baseCachePolicy = createGithubPullRequestCommitsCachePolicy(userId, org, repo, pullNumber)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, org, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubPullRequestCommit[]>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const cachedPullRequestValidator = getCachedValidator(cachedEntry, GITHUB_PULL_REQUEST_COLLECTION_VALIDATOR_KEY)
        const pullRequestResponse = await fetchGithubPullRequestConditionally({
          token: githubToken,
          params: {
            owner: org,
            repo,
            pull_number: pullNumber,
          },
          etag: cachedPullRequestValidator.etag,
          lastModified: cachedPullRequestValidator.lastModified,
        })

        const nextPullRequestValidator = {
          etag: pullRequestResponse.etag ?? cachedPullRequestValidator.etag,
          lastModified: pullRequestResponse.lastModified ?? cachedPullRequestValidator.lastModified,
        } satisfies GithubCacheValidator

        if (pullRequestResponse.notModified) {
          return buildNotModifiedCacheResult(
            GITHUB_PULL_REQUEST_COLLECTION_VALIDATOR_KEY,
            nextPullRequestValidator,
          )
        }

        const commits = await fetchGithubPullRequestCommitsAllPages({
          token: githubToken,
          params: {
            owner: org,
            repo,
            pull_number: pullNumber,
          },
        })

        return buildLoadedCacheResult(
          GITHUB_PULL_REQUEST_COLLECTION_VALIDATOR_KEY,
          nextPullRequestValidator,
          commits.map(mapGithubPullRequestCommit),
        )
      },
    }))
}

async function fetchPullRequestIssueCommentsWithCache(
  userId: string,
  githubToken: string,
  org: string,
  repo: string,
  pullNumber: number,
) {
  const baseCachePolicy = createGithubPullRequestIssueCommentsCachePolicy(userId, org, repo, pullNumber)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, org, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubPullRequestIssueComment[]>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const cachedPagination = getCachedPaginationMetadata(cachedEntry)
        const canUseConditionalComments = canConditionallyRevalidateSinglePageCollection(cachedPagination)
        const params: GithubIssueDetailsCommentParameters = {
          owner: org,
          repo,
          issue_number: pullNumber,
          per_page: 100,
        }

        if (canUseConditionalComments) {
          const response = await fetchGithubRepositoryIssueCommentsConditionally({
            token: githubToken,
            params,
            etag: cachedEntry?.etag,
            lastModified: cachedEntry?.lastModified,
          })

          if (response.notModified) {
            return {
              notModified: true as const,
              etag: response.etag,
              lastModified: response.lastModified,
              pagination: cachedPagination ?? undefined,
            }
          }

          const paginatedComments = await fetchGithubRepositoryIssueCommentsAllPages({
            token: githubToken,
            params: {
              owner: org,
              repo,
              issue_number: pullNumber,
            },
            initialPageItems: response.data!,
          })

          return {
            payload: paginatedComments.items.map(mapGithubPullRequestIssueComment),
            etag: response.etag,
            lastModified: response.lastModified,
            pagination: buildPaginationMetadata(
              paginatedComments.pageCount,
              paginatedComments.itemCount,
              paginatedComments.truncated,
            ),
          }
        }

        const paginatedComments = await fetchGithubRepositoryIssueCommentsAllPages({
          token: githubToken,
          params: {
            owner: org,
            repo,
            issue_number: pullNumber,
          },
        })

        return {
          payload: paginatedComments.items.map(mapGithubPullRequestIssueComment),
          pagination: buildPaginationMetadata(
            paginatedComments.pageCount,
            paginatedComments.itemCount,
            paginatedComments.truncated,
          ),
        }
      },
    }))
}

async function fetchPullRequestReviewsWithCache(
  userId: string,
  githubToken: string,
  org: string,
  repo: string,
  pullNumber: number,
) {
  const baseCachePolicy = createGithubPullRequestReviewsCachePolicy(userId, org, repo, pullNumber)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, org, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubPullRequestReview[]>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: PullRequestReviewsParams = {
          owner: org,
          repo,
          pull_number: pullNumber,
          per_page: 100,
        }

        const response = await fetchGithubPullRequestReviewsConditionally({
          token: githubToken,
          params,
          etag: cachedEntry?.etag,
          lastModified: cachedEntry?.lastModified,
        })

        if (response.notModified) {
          return {
            notModified: true as const,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }

        const reviews = response.data!
          .map(mapGithubPullRequestReview)
          .filter(review =>
            review.state === 'APPROVED'
            || review.state === 'CHANGES_REQUESTED'
            || review.state === 'COMMENTED')

        return {
          payload: reviews,
          etag: response.etag,
          lastModified: response.lastModified,
        }
      },
    }))
}

async function fetchPullRequestCommentsWithCache(
  userId: string,
  githubToken: string,
  org: string,
  repo: string,
  pullNumber: number,
) {
  const baseCachePolicy = createGithubPullRequestCommentsCachePolicy(userId, org, repo, pullNumber)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, org, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubPullRequestReviewComment[]>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const cachedPagination = getCachedPaginationMetadata(cachedEntry)
        const canUseConditionalComments = canConditionallyRevalidateSinglePageCollection(cachedPagination)
        const params: PullRequestCommentsParams = {
          owner: org,
          repo,
          pull_number: pullNumber,
          per_page: 100,
        }

        if (canUseConditionalComments) {
          const response = await fetchGithubPullRequestCommentsConditionally({
            token: githubToken,
            params,
            etag: cachedEntry?.etag,
            lastModified: cachedEntry?.lastModified,
          })

          if (response.notModified) {
            return {
              notModified: true as const,
              etag: response.etag,
              lastModified: response.lastModified,
              pagination: cachedPagination ?? undefined,
            }
          }

          const paginatedComments = await fetchGithubPullRequestCommentsAllPages({
            token: githubToken,
            params: {
              owner: org,
              repo,
              pull_number: pullNumber,
            },
            initialPageItems: response.data!,
          })

          return {
            payload: paginatedComments.items.map(mapGithubPullRequestReviewComment),
            etag: response.etag,
            lastModified: response.lastModified,
            pagination: buildPaginationMetadata(
              paginatedComments.pageCount,
              paginatedComments.itemCount,
              paginatedComments.truncated,
            ),
          }
        }

        const paginatedComments = await fetchGithubPullRequestCommentsAllPages({
          token: githubToken,
          params: {
            owner: org,
            repo,
            pull_number: pullNumber,
          },
        })

        return {
          payload: paginatedComments.items.map(mapGithubPullRequestReviewComment),
          pagination: buildPaginationMetadata(
            paginatedComments.pageCount,
            paginatedComments.itemCount,
            paginatedComments.truncated,
          ),
        }
      },
    }))
}

async function fetchRepositoryDetailsWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
) {
  const baseCachePolicy = createGithubRepositoryDetailsCachePolicy(userId, owner, repo)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, owner, repo)

  const result = await withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubRepositoryDetails>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: GithubRepositoryParameters = {
          owner,
          repo,
        }

        const response = await fetchGithubRepositoryConditionally({
          token: githubToken,
          params,
          etag: cachedEntry?.etag,
          lastModified: cachedEntry?.lastModified,
        })

        if (response.notModified) {
          return {
            notModified: true as const,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }

        const data = response.data!

        return {
          payload: {
            name: data.name,
            full_name: data.full_name,
            private: data.private,
            description: data.description,
            homepage: data.homepage,
            language: data.language,
            default_branch: data.default_branch,
            stargazers_count: data.stargazers_count,
            forks_count: data.forks_count,
            subscribers_count: data.subscribers_count,
            open_issues_count: data.open_issues_count,
            size: data.size,
            pushed_at: data.pushed_at,
            html_url: data.html_url,
            owner: {
              login: data.owner.login,
              name: data.owner.name || undefined,
              avatar_url: data.owner.avatar_url,
            },
            license: data.license
              ? {
                  key: data.license.key,
                  name: data.license.name,
                  spdx_id: data.license.spdx_id,
                }
              : null,
          } satisfies GithubRepositoryDetails,
          etag: response.etag,
          lastModified: response.lastModified,
        }
      },
    }))

  await syncRepositoryPublicVisibility(owner, repo, result.payload.private)

  if (!result.payload.private && cachePolicy.scope !== 'public' && result.cacheStatus !== 'stale') {
    await githubCache.prime({
      ...withGithubPublicScope(baseCachePolicy),
      payload: result.payload,
    })
  }

  return result
}

async function fetchRepositoryReadmeWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
  ref?: string,
) {
  const baseCachePolicy = createGithubRepositoryReadmeCachePolicy(userId, owner, repo, ref)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, owner, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubRepositoryReadme>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: GithubRepositoryReadmeParameters = {
          owner,
          repo,
          ...(ref && ref.trim().length > 0 ? { ref } : {}),
        }

        try {
          const response = await fetchGithubRepositoryReadmeConditionally({
            token: githubToken,
            params,
            etag: cachedEntry?.etag,
            lastModified: cachedEntry?.lastModified,
          })

          if (response.notModified) {
            return {
              notModified: true as const,
              etag: response.etag,
              lastModified: response.lastModified,
            }
          }

          const data = response.data!
          let content: string | null = null
          if (typeof data.content === 'string') {
            const encoding = data.encoding === 'base64' ? 'base64' : 'utf8'
            content = Buffer.from(data.content, encoding).toString('utf8')
          }
          const path = typeof data.path === 'string' ? data.path : null

          return {
            payload: {
              content,
              path,
            } satisfies GithubRepositoryReadme,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }
        catch (error) {
          const status = (error as { status?: number }).status
          if (status === 404) {
            return {
              payload: { content: null, path: null } satisfies GithubRepositoryReadme,
            }
          }

          throw error
        }
      },
    }))
}

async function fetchRepositoryBranchesWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
) {
  const baseCachePolicy = createGithubRepositoryBranchesCachePolicy(userId, owner, repo)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, owner, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubRepositoryBranch[]>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: GithubRepositoryBranchesParameters = {
          owner,
          repo,
          per_page: 100,
        }

        const response = await fetchGithubRepositoryBranchesConditionally({
          token: githubToken,
          params,
          etag: cachedEntry?.etag,
          lastModified: cachedEntry?.lastModified,
        })

        if (response.notModified) {
          return {
            notModified: true as const,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }

        return {
          payload: response.data!.map(branch => ({
            name: branch.name,
            commit: {
              sha: branch.commit.sha,
              url: branch.commit.url,
            },
            protected: branch.protected,
          } satisfies GithubRepositoryBranch)),
          etag: response.etag,
          lastModified: response.lastModified,
        }
      },
    }))
}

async function fetchRepositoryPullRequestsWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
  filters: GithubPullRequestSearchFilters,
) {
  const repositoryFilter = normalizeRepositoryFilter(`${owner}/${repo}`)
  if (!repositoryFilter) {
    throw new Error('Missing repository')
  }

  const searchFilters = {
    ...filters,
    repos: [repositoryFilter],
  }
  const query = buildPullRequestSearchQuery(searchFilters, { openOnly: false })
  const baseCachePolicy = createGithubRepositoryPullRequestsCachePolicy(
    userId,
    owner,
    repo,
    normalizePullRequestSearchCacheKey(searchFilters),
  )
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, owner, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubPullRequest[]>({
      ...cachePolicy,
      load: async () => {
        const nodes = await fetchGithubPullRequestSearchGraphql({
          token: githubToken,
          query,
          limit: REPOSITORY_PULL_REQUESTS_LIMIT,
        })

        return {
          payload: nodes.map(node => mapGithubGraphqlPullRequest(node).pullRequest),
        }
      },
    }))
}

async function fetchBranchPullRequest(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
  branch: string,
) {
  return withGithubMetrics(userId, 'pull_request.branch_lookup', async () => {
    const params: ListPullsParams = {
      owner,
      repo,
      state: 'open',
      head: `${owner}:${branch}`,
      per_page: 1,
    }

    const pullRequests = await fetchGithubPullRequests({ token: githubToken, params })
    return pullRequests.at(0) ? mapGithubPullRequest(pullRequests[0]) : null
  })
}

async function fetchRepositoryIssuesWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
) {
  const baseCachePolicy = createGithubRepositoryIssuesCachePolicy(userId, owner, repo)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, owner, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubIssue[]>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: GithubIssueParameters = {
          owner,
          repo,
          state: 'all',
          sort: 'updated',
          direction: 'desc',
          per_page: 20,
        }

        const response = await fetchGithubRepositoryIssuesConditionally({
          token: githubToken,
          params,
          etag: cachedEntry?.etag,
          lastModified: cachedEntry?.lastModified,
        })

        if (response.notModified) {
          return {
            notModified: true as const,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }

        return {
          payload: (response.data || []).filter(issue => !issue.pull_request).map(issue => ({
            id: issue.id,
            number: issue.number,
            title: issue.title,
            state: issue.state,
            state_reason: issue.state_reason,
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            closed_at: issue.closed_at,
            labels: issue.labels,
            comments_count: issue.comments,
            user: formatGithubUser(issue.user),
            repository: {
              owner,
              repo,
            },
          } satisfies GithubIssue)),
          etag: response.etag,
          lastModified: response.lastModified,
        }
      },
    }))
}

async function fetchRepositoryIssueDetailsWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
  issueNumber: number,
) {
  const baseCachePolicy = createGithubRepositoryIssueDetailsCachePolicy(userId, owner, repo, issueNumber)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, owner, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubIssueDetails>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const paramsIssue: GithubIssueDetailsParameters = {
          owner,
          repo,
          issue_number: issueNumber,
        }

        const paramsComments: GithubIssueDetailsCommentParameters = {
          owner,
          repo,
          issue_number: issueNumber,
          per_page: 100,
        }

        const cachedIssueValidator = cachedEntry?.validators?.[GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY] ?? {
          etag: cachedEntry?.etag,
          lastModified: cachedEntry?.lastModified,
        }
        const cachedIssueCommentsValidator = cachedEntry?.validators?.[GITHUB_ISSUE_DETAILS_COMMENTS_VALIDATOR_KEY]
        const cachedCommentsPagination = getCachedPaginationMetadata(cachedEntry)
        const canUseConditionalIssueComments = canConditionallyRevalidateSinglePageCollection(cachedCommentsPagination)

        const issueResponse = await fetchGithubRepositoryIssueConditionally({
          token: githubToken,
          params: paramsIssue,
          etag: cachedIssueValidator.etag,
          lastModified: cachedIssueValidator.lastModified,
        })

        const issueCommentsResponse = canUseConditionalIssueComments
          ? await fetchGithubRepositoryIssueCommentsConditionally({
              token: githubToken,
              params: paramsComments,
              etag: cachedIssueCommentsValidator?.etag,
              lastModified: cachedIssueCommentsValidator?.lastModified,
            })
          : null

        const paginatedIssueComments = issueCommentsResponse?.notModified
          ? null
          : await fetchGithubRepositoryIssueCommentsAllPages({
              token: githubToken,
              params: {
                owner,
                repo,
                issue_number: issueNumber,
              },
              ...(issueCommentsResponse?.data
                ? { initialPageItems: issueCommentsResponse.data }
                : {}),
            })

        const nextValidators = buildGithubIssueDetailsValidators({
          issue: {
            etag: issueResponse.etag ?? cachedIssueValidator.etag,
            lastModified: issueResponse.lastModified ?? cachedIssueValidator.lastModified,
          },
          issueComments: {
            etag: issueCommentsResponse?.etag ?? cachedIssueCommentsValidator?.etag,
            lastModified: issueCommentsResponse?.lastModified ?? cachedIssueCommentsValidator?.lastModified,
          },
        })

        if (issueResponse.notModified && issueCommentsResponse?.notModified) {
          return {
            notModified: true as const,
            etag: nextValidators[GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY]?.etag,
            lastModified: nextValidators[GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY]?.lastModified,
            validators: nextValidators,
            pagination: cachedCommentsPagination ?? undefined,
          }
        }

        if (!cachedEntry && (issueResponse.notModified || issueCommentsResponse?.notModified)) {
          throw new Error('GitHub issue details revalidation requires an existing cache entry')
        }

        const payload = mergeGithubIssueDetailsPayload({
          owner,
          repo,
          cachedPayload: cachedEntry?.payload ?? null,
          issue: issueResponse.notModified ? null : issueResponse.data!,
          issueComments: paginatedIssueComments?.items ?? null,
        })

        return {
          payload,
          etag: nextValidators[GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY]?.etag,
          lastModified: nextValidators[GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY]?.lastModified,
          validators: nextValidators,
          pagination: paginatedIssueComments
            ? buildPaginationMetadata(
                paginatedIssueComments.pageCount,
                paginatedIssueComments.itemCount,
                paginatedIssueComments.truncated,
              )
            : cachedCommentsPagination ?? undefined,
        }
      },
    }))
}

async function fetchRepositoryTreeWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
  treeSha: string,
  recursive?: string,
) {
  const baseCachePolicy = createGithubRepositoryTreeCachePolicy(userId, owner, repo, treeSha, recursive)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, owner, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubRepositoryTree>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: GithubRepositoryTreeParams = {
          owner,
          repo,
          tree_sha: treeSha,
          ...(recursive !== undefined ? { recursive } : {}),
        }

        const response = await fetchGithubRepositoryTreesConditionally({
          token: githubToken,
          params,
          etag: cachedEntry?.etag,
          lastModified: cachedEntry?.lastModified,
        })

        if (response.notModified) {
          return {
            notModified: true as const,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }

        const data = response.data!

        return {
          payload: {
            sha: data.sha,
            url: data.url,
            truncated: data.truncated,
            tree: data.tree,
          } satisfies GithubRepositoryTree,
          etag: response.etag,
          lastModified: response.lastModified,
        }
      },
    }))
}

async function fetchRepositoryFileWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
  path: string,
  ref: string,
) {
  const baseCachePolicy = createGithubRepositoryFileCachePolicy(userId, owner, repo, path, ref)
  const cachePolicy = await resolveRepositoryReadCachePolicy(baseCachePolicy, owner, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubFileContent>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: GetContentParams = {
          owner,
          repo,
          path,
          ref,
        }

        try {
          const response = await fetchGithubRepositoryContentConditionally({
            token: githubToken,
            params,
            etag: cachedEntry?.etag,
            lastModified: cachedEntry?.lastModified,
          })

          if (response.notModified) {
            return {
              notModified: true as const,
              etag: response.etag,
              lastModified: response.lastModified,
            }
          }

          const data = response.data!

          let content: string | null = null

          if (typeof data === 'string') {
            content = data
          }
          else if (Buffer.isBuffer(data)) {
            content = data.toString('utf8')
          }
          else if (data && typeof data === 'object' && 'content' in data) {
            const payload = data as { content?: string, encoding?: string }
            if (typeof payload.content === 'string') {
              const encoding = payload.encoding === 'base64' ? 'base64' : 'utf8'
              content = Buffer.from(payload.content, encoding).toString('utf8')
            }
          }

          return {
            payload: { content } satisfies GithubFileContent,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }
        catch (error) {
          const status = (error as { status?: number }).status
          if (status === 404) {
            return {
              payload: { content: null } satisfies GithubFileContent,
            }
          }

          throw error
        }
      },
    }))
}

function encodeGithubFileAsset(data: unknown): string | null {
  if (Buffer.isBuffer(data)) {
    return data.toString('base64')
  }
  if (data && typeof data === 'object' && 'content' in data) {
    const payload = data as { content?: string, encoding?: string }
    if (typeof payload.content === 'string' && payload.encoding === 'base64') {
      return payload.content.replace(/\n/g, '')
    }
  }
  return null
}

async function fetchRepositoryFileAssetWithCache(
  userId: string,
  githubToken: string,
  owner: string,
  repo: string,
  path: string,
  ref: string,
) {
  const baseCachePolicy = createGithubRepositoryFileCachePolicy(userId, owner, repo, path, ref)
  const cachePolicy = await resolveRepositoryReadCachePolicy({
    ...baseCachePolicy,
    operation: `${baseCachePolicy.operation}.asset`,
    resourceKey: `${baseCachePolicy.resourceKey}:asset`,
  }, owner, repo)

  return withGithubMetrics(userId, cachePolicy.operation, () =>
    githubCache.getOrLoad<GithubFileAsset>({
      ...cachePolicy,
      load: async ({ cachedEntry }) => {
        const params: GetContentParams = {
          owner,
          repo,
          path,
          ref,
        }

        try {
          const response = await fetchGithubRepositoryContentObjectConditionally({
            token: githubToken,
            params,
            etag: cachedEntry?.etag,
            lastModified: cachedEntry?.lastModified,
          })

          if (response.notModified) {
            return {
              notModified: true as const,
              etag: response.etag,
              lastModified: response.lastModified,
            }
          }

          const contentBase64 = encodeGithubFileAsset(response.data)

          return {
            payload: { contentBase64 } satisfies GithubFileAsset,
            etag: response.etag,
            lastModified: response.lastModified,
          }
        }
        catch (error) {
          const status = (error as { status?: number }).status
          if (status === 404) {
            return {
              payload: { contentBase64: null } satisfies GithubFileAsset,
            }
          }

          throw error
        }
      },
    }))
}

async function invalidateGithubCacheTags(tags: string[]) {
  try {
    await githubCache.invalidateTags(tags)
  }
  catch (error) {
    logger.warn({ error, tags }, 'Failed to invalidate GitHub cache tags')
  }
}

function sleep(ms: number) {
  return new Promise(resolve => setTimeout(resolve, ms))
}

async function waitForPullRequestHeadShaChange(
  token: string,
  params: PullRequestParams,
  previousHeadSha: string,
) {
  for (let attempt = 0; attempt < GITHUB_UPDATE_BRANCH_POLL_ATTEMPTS; attempt += 1) {
    if (attempt > 0) {
      await sleep(GITHUB_UPDATE_BRANCH_POLL_INTERVAL_MS)
    }

    const pullRequest = await fetchGithubPullRequest({
      token,
      params,
    })

    if (pullRequest.head.sha !== previousHeadSha) {
      return pullRequest.head.sha
    }
  }

  return null
}

const githubRouter = new Hono()

githubRouter.use('*', authMiddlewarePro)

export const githubRoutes = githubRouter
  .get('/notifications', async (ctx) => {
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchNotificationsWithCache(user.id, githubToken)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ notifications: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .delete('/notifications/:threadId/done', async (ctx) => {
    const threadId = ctx.req.param('threadId')
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      await markGithubNotificationDone({ token: githubToken, threadId })
      await invalidateGithubCacheTags([getGithubNotificationsTag(user.id)])
      return ctx.json({ success: true }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .patch('/notifications/:threadId/read', async (ctx) => {
    const threadId = ctx.req.param('threadId')
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      await markGithubNotificationRead({ token: githubToken, threadId })
      await invalidateGithubCacheTags([getGithubNotificationsTag(user.id)])
      return ctx.json({ success: true }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/search', zValidator(
    'json',
    pullRequestSearchBodySchema,
  ), async (ctx) => {
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken
    const { filters } = ctx.req.valid('json')
    try {
      const result = await fetchPullRequestsSearchWithCache(
        user.id,
        githubToken,
        filters,
      )
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ pullRequests: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/filter-options', zValidator(
    'json',
    pullRequestFilterOptionsBodySchema,
  ), async (ctx) => {
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken
    const { repos } = ctx.req.valid('json')

    try {
      const options = await withGithubMetrics(user.id, 'viewer.pull_requests.filter_options', () =>
        fetchPullRequestFilterOptions(githubToken, repos))
      return ctx.json({ options }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchPullRequestDetailsWithCache(user.id, githubToken, org, repo, pullNumber)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ pullRequest: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .patch('/pr/:id', zValidator(
    'json',
    updateDescriptionBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const { body } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: UpdatePullRequestParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        body,
      }

      const data = await patchGithubPullRequest({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))
      const pullRequest = mapGithubPullRequestDescriptionUpdate(data)
      return ctx.json({ pullRequest }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:id/assignees', zValidator(
    'json',
    pullRequestUsersMutationBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const { users } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: AddIssueAssigneesParams = {
        owner: org,
        repo,
        issue_number: pullNumber,
        assignees: users,
      }

      await withGithubMetrics(user.id, 'pull_request.assignees.add', () =>
        addGithubIssueAssignees({
          token: githubToken,
          params,
        }))

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      return ctx.body(null, 204)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .delete('/pr/:id/assignees', zValidator(
    'json',
    pullRequestUsersMutationBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const { users } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: RemoveIssueAssigneesParams = {
        owner: org,
        repo,
        issue_number: pullNumber,
        assignees: users,
      }

      await withGithubMetrics(user.id, 'pull_request.assignees.remove', () =>
        removeGithubIssueAssignees({
          token: githubToken,
          params,
        }))

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      return ctx.body(null, 204)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:id/labels', zValidator(
    'json',
    pullRequestLabelsMutationBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const { labels } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: AddIssueLabelsParams = {
        owner: org,
        repo,
        issue_number: pullNumber,
        labels,
      }

      await withGithubMetrics(user.id, 'pull_request.labels.add', () =>
        addGithubIssueLabels({
          token: githubToken,
          params,
        }))

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      return ctx.body(null, 204)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .delete('/pr/:id/labels', zValidator(
    'json',
    pullRequestLabelsMutationBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const { labels } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      await withGithubMetrics(user.id, 'pull_request.labels.remove', async () => {
        await Promise.all(labels.map(label =>
          removeGithubIssueLabel({
            token: githubToken,
            params: {
              owner: org,
              repo,
              issue_number: pullNumber,
              name: label,
            },
          })))
      })

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      return ctx.body(null, 204)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:id/requested-reviewers', zValidator(
    'json',
    pullRequestUsersMutationBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const { users } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: RequestPullRequestReviewersParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        reviewers: users,
      }

      await withGithubMetrics(user.id, 'pull_request.review_requests.add', () =>
        requestGithubPullRequestReviewers({
          token: githubToken,
          params,
        }))

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      return ctx.body(null, 204)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .delete('/pr/:id/requested-reviewers', zValidator(
    'json',
    pullRequestUsersMutationBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const { users } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: RemovePullRequestReviewersParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        reviewers: users,
      }

      await withGithubMetrics(user.id, 'pull_request.review_requests.remove', () =>
        removeGithubPullRequestReviewers({
          token: githubToken,
          params,
        }))

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      return ctx.body(null, 204)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:id/ready-for-review', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const payload = await ctx.req.json().catch(() => ({}))
    const parsedBody = pullRequestStatusMutationBodySchema.safeParse(payload)

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }
    if (!parsedBody.success) {
      const firstIssue = parsedBody.error.issues[0]
      const message = firstIssue?.path[0] === 'pullRequestId'
        ? (firstIssue.message === 'Invalid input: expected string, received undefined'
            ? 'Missing pull request id'
            : firstIssue.message)
        : 'Invalid pull request status payload'
      return ctx.json({ error: message }, 400)
    }

    const { pullRequestId } = parsedBody.data

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      await withGithubMetrics(user.id, 'pull_request.ready_for_review', () =>
        markGithubPullRequestReadyForReview({
          token: githubToken,
          pullRequestId,
        }))

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      return ctx.body(null, 204)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:id/convert-to-draft', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const payload = await ctx.req.json().catch(() => ({}))
    const parsedBody = pullRequestStatusMutationBodySchema.safeParse(payload)

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }
    if (!parsedBody.success) {
      const firstIssue = parsedBody.error.issues[0]
      const message = firstIssue?.path[0] === 'pullRequestId'
        ? (firstIssue.message === 'Invalid input: expected string, received undefined'
            ? 'Missing pull request id'
            : firstIssue.message)
        : 'Invalid pull request status payload'
      return ctx.json({ error: message }, 400)
    }

    const { pullRequestId } = parsedBody.data

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      await withGithubMetrics(user.id, 'pull_request.convert_to_draft', () =>
        convertGithubPullRequestToDraft({
          token: githubToken,
          pullRequestId,
        }))

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      return ctx.body(null, 204)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/merge-readiness', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const mergeReadiness = await withGithubMetrics(user.id, 'pull_request.merge_readiness', () =>
        fetchGithubPullRequestMergeReadiness({
          token: githubToken,
          params: {
            owner: org,
            repo,
            pull_number: pullNumber,
          },
        }))

      return ctx.json({ mergeReadiness } satisfies { mergeReadiness: GithubPullRequestMergeReadiness }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }

      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/checks', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const checks = await withGithubMetrics(user.id, 'pull_request.checks', () =>
        fetchGithubPullRequestChecksSummary({
          token: githubToken,
          params: {
            owner: org,
            repo,
            pull_number: pullNumber,
          },
        }))

      return ctx.json({ checks } satisfies { checks: GithubPullRequestChecksSummary }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }

      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .put('/pr/:id/merge', zValidator(
    'json',
    mergePullRequestBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const {
      method,
      expectedHeadSha,
      commitTitle,
      commitMessage,
    } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken
    const trimmedCommitTitle = commitTitle?.trim()
    const trimmedCommitMessage = commitMessage?.trim()

    try {
      const params: MergePullRequestParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        sha: expectedHeadSha,
        merge_method: method,
        ...(trimmedCommitTitle ? { commit_title: trimmedCommitTitle } : {}),
        ...(trimmedCommitMessage ? { commit_message: trimmedCommitMessage } : {}),
      }

      const data = await withGithubMetrics(user.id, 'pull_request.merge', () =>
        mergeGithubPullRequest({
          token: githubToken,
          params,
        }))

      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
      }))

      const mergeResult = {
        merged: data.merged,
        sha: data.sha,
        message: data.message,
        method,
      } satisfies GithubPullRequestMergeResult

      return ctx.json({ mergeResult }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 405 || status === 409 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }

      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .put('/pr/:id/update-branch', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const pullRequestParams: PullRequestParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
      }
      const pullRequestBeforeUpdate = await withGithubMetrics(user.id, 'pull_request.update_branch.current_head', () =>
        fetchGithubPullRequest({
          token: githubToken,
          params: pullRequestParams,
        }))
      const params: UpdatePullRequestBranchParams = {
        ...pullRequestParams,
        expected_head_sha: pullRequestBeforeUpdate.head.sha,
      }

      await withGithubMetrics(user.id, 'pull_request.update_branch', () =>
        updateGithubPullRequestBranch({
          token: githubToken,
          params,
        }))

      try {
        await withGithubMetrics(user.id, 'pull_request.update_branch.wait', () =>
          waitForPullRequestHeadShaChange(
            githubToken,
            pullRequestParams,
            pullRequestBeforeUpdate.head.sha,
          ))
      }
      catch (error) {
        logger.warn(
          { error, owner: org, repo, pullNumber },
          'Failed to wait for GitHub pull request branch update',
        )
      }

      await invalidateGithubCacheTags([
        ...getGithubPullRequestMutationTags({
          userId: user.id,
          owner: org,
          repo,
          pullNumber,
        }),
        getGithubPullRequestCommitsTag(org, repo, pullNumber),
        getGithubPullRequestFilesTag(org, repo, pullNumber),
      ])

      return ctx.body(null, 202)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }

      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/files', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const commitSha = ctx.req.query('commitSha')?.trim()

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchPullRequestFilesWithCache(
        user.id,
        githubToken,
        org,
        repo,
        pullNumber,
        commitSha,
      )
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ files: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/commits', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchPullRequestCommitsWithCache(user.id, githubToken, org, repo, pullNumber)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ commits: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/issue-comments', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchPullRequestIssueCommentsWithCache(user.id, githubToken, org, repo, pullNumber)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ comments: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/reviews', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchPullRequestReviewsWithCache(user.id, githubToken, org, repo, pullNumber)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ reviews: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/comments', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchPullRequestCommentsWithCache(user.id, githubToken, org, repo, pullNumber)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ comments: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:id/reviews', zValidator(
    'json',
    createPullRequestReviewBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const { event, body } = ctx.req.valid('json')
    const trimmedBody = body?.trim()

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const bodyRequired = event === 'COMMENT' || event === 'REQUEST_CHANGES'
    if (bodyRequired && !trimmedBody) {
      return ctx.json({ error: 'Missing review body' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: CreatePullRequestReviewParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        event,
        ...(trimmedBody ? { body: trimmedBody } : {}),
      }

      const data = await createGithubPullRequestReview({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
        includeReviews: true,
      }))
      const review = mapGithubPullRequestReview(data)
      return ctx.json({ review }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:id/comments', zValidator(
    'json',
    createPullRequestLineCommentBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const {
      body,
      path,
      commitId,
      line,
      side,
      startLine,
      startSide,
    } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: CreatePullRequestCommentParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        body,
        path,
        commit_id: commitId,
        line,
        side,
        ...(startLine != null ? { start_line: startLine } : {}),
        ...(startSide != null ? { start_side: startSide } : {}),
      }

      const data = await createGithubPullRequestComment({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
        includeComments: true,
      }))

      const comment = mapGithubPullRequestReviewComment(data)
      return ctx.json({ comment }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:prId/comments/:commentId/replies', zValidator(
    'json',
    createPullRequestThreadReplyBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('prId'))
    const inReplyToId = Number(ctx.req.param('commentId'))
    const { body } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber) || Number.isNaN(inReplyToId)) {
      return ctx.json({ error: 'Missing org, repo, prId, or commentId' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: CreatePullRequestCommentReplyParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        comment_id: inReplyToId,
        body,
      }

      const data = await createGithubPullRequestCommentReply({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
        includeComments: true,
      }))

      const comment = mapGithubPullRequestReviewComment(data)
      return ctx.json({ comment }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .delete('/pr/:id/comments/:commentId', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const commentId = Number(ctx.req.param('commentId'))

    if (!org || !repo || Number.isNaN(pullNumber) || Number.isNaN(commentId)) {
      return ctx.json({ error: 'Missing org, repo, id, or commentId' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: DeletePullRequestCommentParams = {
        owner: org,
        repo,
        comment_id: commentId,
      }

      await deleteGithubPullRequestComment({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
        includeComments: true,
      }))

      return ctx.json({ success: true }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .patch('/pr/:id/comments/:commentId', zValidator(
    'json',
    updatePullRequestCommentBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const commentId = Number(ctx.req.param('commentId'))
    const { body } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber) || Number.isNaN(commentId)) {
      return ctx.json({ error: 'Missing org, repo, id, or commentId' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: UpdatePullRequestCommentParams = {
        owner: org,
        repo,
        comment_id: commentId,
        body,
      }

      const data = await patchGithubPullRequestComment({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner: org,
        repo,
        pullNumber,
        includeComments: true,
      }))

      const comment = mapGithubPullRequestReviewComment(data)
      return ctx.json({ comment }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/file', async (ctx) => {
    const { org, repo, path, ref } = ctx.req.query()

    if (!org || !repo || !path || !ref) {
      return ctx.json({ error: 'Missing org, repo, path, or ref' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchRepositoryFileWithCache(user.id, githubToken, org, repo, path, ref)
      setGithubCacheHeaders(ctx, result)
      return ctx.json(result.payload, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ content: null } satisfies GithubFileContent, 200)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/file/asset', async (ctx) => {
    const { org, repo, path, ref } = ctx.req.query()

    if (!org || !repo || !path || !ref) {
      return ctx.json({ error: 'Missing org, repo, path, or ref' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchRepositoryFileAssetWithCache(
        user.id,
        githubToken,
        org,
        repo,
        path,
        ref,
      )
      setGithubCacheHeaders(ctx, result)

      return ctx.json(result.payload, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ contentBase64: null } satisfies GithubFileAsset, 200)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/me', async (ctx) => {
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchUserRepositoriesWithCache(user.id, githubToken)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ repositories: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchRepositoryDetailsWithCache(user.id, githubToken, owner, repo)
      setGithubCacheHeaders(ctx, result)
      return ctx.json(result.payload, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ error: 'Repository not found' }, 404)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/readme', async (ctx) => {
    const { owner, repo } = ctx.req.param()
    const ref = ctx.req.query('ref')

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchRepositoryReadmeWithCache(user.id, githubToken, owner, repo, ref)
      setGithubCacheHeaders(ctx, result)
      return ctx.json(result.payload, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ content: null, path: null } satisfies GithubRepositoryReadme, 200)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/branches', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchRepositoryBranchesWithCache(user.id, githubToken, owner, repo)
      setGithubCacheHeaders(ctx, result)
      return ctx.json(result.payload, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ error: 'Repository not found' }, 404)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/trees/:tree_sha', async (ctx) => {
    const { owner, repo, tree_sha } = ctx.req.param()
    const recursive = ctx.req.query('recursive')

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchRepositoryTreeWithCache(user.id, githubToken, owner, repo, tree_sha, recursive)
      setGithubCacheHeaders(ctx, result)
      return ctx.json(result.payload, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ error: 'Repository not found' }, 404)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/pr', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    const filtersResult = pullRequestSearchFiltersSchema.safeParse(
      repositoryPullRequestFiltersInput(ctx.req.url),
    )
    if (!filtersResult.success) {
      const message = filtersResult.error.issues[0]?.message || 'Invalid pull request filters'
      return ctx.json({ error: message }, 400)
    }

    try {
      const result = await fetchRepositoryPullRequestsWithCache(
        user.id,
        githubToken,
        owner,
        repo,
        filtersResult.data,
      )
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ pullRequests: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/repos/:owner/:repo/pr', async (ctx) => {
    const { owner, repo } = ctx.req.param()
    const branch = ctx.req.query('branch')?.trim()
    const payload = await ctx.req.json().catch(() => null)
    const parsedBody = createPullRequestBodySchema.safeParse(payload)

    if (!branch) {
      return ctx.json({ error: 'Missing branch' }, 400)
    }

    if (!parsedBody.success) {
      const message = parsedBody.error.issues[0]?.message || 'Invalid pull request payload'
      return ctx.json({ error: message }, 400)
    }

    const { title, base, body, draft } = parsedBody.data

    const trimmedBody = body?.trim()
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: CreatePullRequestParams = {
        owner,
        repo,
        head: `${owner}:${branch}`,
        base,
        title,
        ...(trimmedBody ? { body: trimmedBody } : {}),
        ...(draft === true ? { draft: true } : {}),
      }

      const data = await createGithubPullRequest({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubPullRequestMutationTags({
        userId: user.id,
        owner,
        repo,
        pullNumber: data.number,
      }))
      const pullRequest = mapGithubPullRequest(data)
      return ctx.json({ pullRequest }, 201)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/pr/branch', async (ctx) => {
    const { owner, repo } = ctx.req.param()
    const branch = ctx.req.query('branch')?.trim()

    if (!branch) {
      return ctx.json({ error: 'Missing branch' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const pullRequest = await fetchBranchPullRequest(user.id, githubToken, owner, repo, branch)
      return ctx.json({ pullRequest }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/issues', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchRepositoryIssuesWithCache(user.id, githubToken, owner, repo)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ issues: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/issues/:issue_number', async (ctx) => {
    const { owner, repo, issue_number } = ctx.req.param()

    const issueNumber = Number(issue_number)

    if (!owner || !repo || Number.isNaN(issueNumber)) {
      return ctx.json({ error: 'Missing owner, repo, or issue number' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const result = await fetchRepositoryIssueDetailsWithCache(user.id, githubToken, owner, repo, issueNumber)
      setGithubCacheHeaders(ctx, result)
      return ctx.json({ issue: result.payload }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .patch('/repos/:owner/:repo/issues/:issue_number', zValidator(
    'json',
    updateDescriptionBodySchema,
  ), async (ctx) => {
    const { owner, repo, issue_number } = ctx.req.param()
    const issueNumber = Number(issue_number)
    const { body } = ctx.req.valid('json')

    if (!owner || !repo || Number.isNaN(issueNumber)) {
      return ctx.json({ error: 'Missing owner, repo, or issue number' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: UpdateIssueParams = {
        owner,
        repo,
        issue_number: issueNumber,
        body,
      }

      const data = await patchGithubIssue({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubIssueMutationTags({
        owner,
        repo,
        issueNumber,
      }))
      const issue = mapGithubIssueDescriptionUpdate(data)
      return ctx.json({ issue }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/repos/:owner/:repo/issues/:issue_number/comments', zValidator(
    'json',
    issueCommentBodySchema,
  ), async (ctx) => {
    const { owner, repo, issue_number } = ctx.req.param()
    const issueNumber = Number(issue_number)
    const { body } = ctx.req.valid('json')

    if (!owner || !repo || Number.isNaN(issueNumber)) {
      return ctx.json({ error: 'Missing owner, repo, or issue number' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: CreateIssueCommentParams = {
        owner,
        repo,
        issue_number: issueNumber,
        body,
      }

      const data = await createGithubIssueComment({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubIssueMutationTags({
        owner,
        repo,
        issueNumber,
        includeComments: true,
      }))
      const comment = mapGithubIssueComment(data)
      return ctx.json({ comment }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .patch('/repos/:owner/:repo/issues/:issue_number/comments/:comment_id', zValidator(
    'json',
    issueCommentBodySchema,
  ), async (ctx) => {
    const { owner, repo, issue_number, comment_id } = ctx.req.param()
    const issueNumber = Number(issue_number)
    const commentId = Number(comment_id)
    const { body } = ctx.req.valid('json')

    if (!owner || !repo || Number.isNaN(issueNumber) || Number.isNaN(commentId)) {
      return ctx.json({ error: 'Missing owner, repo, issue number, or comment id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: UpdateIssueCommentParams = {
        owner,
        repo,
        comment_id: commentId,
        body,
      }

      const data = await patchGithubIssueComment({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubIssueMutationTags({
        owner,
        repo,
        issueNumber,
        includeComments: true,
      }))
      const comment = mapGithubIssueComment(data)
      return ctx.json({ comment }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .delete('/repos/:owner/:repo/issues/:issue_number/comments/:comment_id', async (ctx) => {
    const { owner, repo, issue_number, comment_id } = ctx.req.param()
    const issueNumber = Number(issue_number)
    const commentId = Number(comment_id)

    if (!owner || !repo || Number.isNaN(issueNumber) || Number.isNaN(commentId)) {
      return ctx.json({ error: 'Missing owner, repo, issue number, or comment id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: DeleteIssueCommentParams = {
        owner,
        repo,
        comment_id: commentId,
      }

      await deleteGithubIssueComment({ token: githubToken, params })
      await invalidateGithubCacheTags(getGithubIssueMutationTags({
        owner,
        repo,
        issueNumber,
        includeComments: true,
      }))
      return ctx.json({ success: true }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/asset/resolve', async (ctx) => {
    const url = ctx.req.query('url')
    if (!url || !url.startsWith('https://github.com/user-attachments/assets/')) {
      return ctx.json({ error: 'Invalid or missing url parameter' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const response = await fetch(url, {
        method: 'HEAD',
        headers: { Authorization: `Bearer ${githubToken}` },
        redirect: 'manual',
      })

      const location = response.headers.get('location')
      if (!location) {
        return ctx.json({ error: 'GitHub did not redirect to signed URL' }, 502)
      }

      return ctx.json({ url: location }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
