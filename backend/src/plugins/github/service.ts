import type { Endpoints, RequestHeaders, RequestParameters } from '@octokit/types'
import type {
  CommitFileResponse,
  CommitParams,
  CommitResponse,
  CompareParams,
  CreateIssueCommentParams,
  CreateIssueCommentResponse,
  CreatePullRequestCommentParams,
  CreatePullRequestCommentReplyParams,
  CreatePullRequestCommentReplyResponse,
  CreatePullRequestCommentResponse,
  CreatePullRequestReviewParams,
  CreatePullRequestReviewResponse,
  DeleteIssueCommentParams,
  DeletePullRequestCommentParams,
  GetContentParams,
  GetContentResponse,
  GithubIssueDetailsCommentParameters,
  GithubIssueDetailsCommentResponse,
  GithubIssueDetailsParameters,
  GithubIssueDetailsResponse,
  GithubIssueParameters,
  GithubIssueResponse,
  GithubRepositoryBranchesParameters,
  GithubRepositoryBranchesResponse,
  GithubRepositoryParameters,
  GithubRepositoryReadmeParameters,
  GithubRepositoryReadmeResponse,
  GithubRepositoryResponse,
  GithubRepositoryTreeParams,
  GithubRepositoryTreesResponse,
  GithubUserResponse,
  ListPullsParams,
  NotificationResponse,
  NotificationsParams,
  PullRequestCommentResponse,
  PullRequestCommentsParams,
  PullRequestCommitResponse,
  PullRequestCommitsParams,
  PullRequestDetailsResponse,
  PullRequestFileResponse,
  PullRequestFilesParams,
  PullRequestParams,
  PullRequestResponse,
  PullRequestReviewResponse,
  PullRequestReviewsParams,
  SearchIssuesParams,
  SearchIssuesResponse,
  UpdateIssueCommentParams,
  UpdateIssueCommentResponse,
  UpdateIssueParams,
  UpdateIssueResponse,
  UpdatePullRequestCommentParams,
  UpdatePullRequestCommentResponse,
  UpdatePullRequestParams,
  UpdatePullRequestResponse,
  UserRepositoriesParams,
  UserRepositoryResponse,
} from './types.js'
import { request } from '@octokit/request'
import { logger } from '../../lib/logger.js'
import { getGithubMetricsContext } from './metrics/github-metrics-context.js'
import { githubMetricsCollector } from './metrics/github-metrics.js'

function githubAuthHeaders(token: string, extraHeaders?: Record<string, string>): RequestHeaders {
  return {
    authorization: `Bearer ${token}`,
    ...extraHeaders,
  }
}

export interface GithubConditionalRequestOptions<Route extends keyof Endpoints> {
  token: string
  params: Endpoints[Route]['parameters']
  etag?: string
  lastModified?: string
  headers?: Record<string, string>
}

export interface GithubConditionalResponse<Route extends keyof Endpoints> {
  data: Endpoints[Route]['response']['data'] | null
  notModified: boolean
  etag?: string
  lastModified?: string
}

export interface GithubRateLimitInfo {
  limit?: number
  remaining?: number
  used?: number
  reset?: number
  resource?: string
}

const GITHUB_RATE_LIMIT_NEAR_THRESHOLD = 0.1

interface GithubErrorLike {
  status?: number
  response?: {
    headers?: GithubResponseHeaders
  }
}

type GithubResponseHeaders = Record<string, string | number | string[] | undefined>
type GithubRequestOptions<Route extends keyof Endpoints> = Route extends keyof Endpoints
  ? Endpoints[Route]['parameters'] & RequestParameters
  : RequestParameters

function readGithubHeader(
  headers: GithubResponseHeaders | undefined,
  name: string,
) {
  const headerValue = headers?.[name]
  if (Array.isArray(headerValue)) {
    return headerValue[0]
  }

  if (typeof headerValue === 'number') {
    return String(headerValue)
  }

  return headerValue
}

function parseGithubNumericHeader(value: string | undefined) {
  if (!value) {
    return undefined
  }

  const parsedValue = Number.parseInt(value, 10)
  return Number.isNaN(parsedValue) ? undefined : parsedValue
}

export function extractGithubRateLimitInfo(headers: GithubResponseHeaders | undefined): GithubRateLimitInfo | null {
  const limit = parseGithubNumericHeader(readGithubHeader(headers, 'x-ratelimit-limit'))
  const remaining = parseGithubNumericHeader(readGithubHeader(headers, 'x-ratelimit-remaining'))
  const used = parseGithubNumericHeader(readGithubHeader(headers, 'x-ratelimit-used'))
  const reset = parseGithubNumericHeader(readGithubHeader(headers, 'x-ratelimit-reset'))
  const resource = readGithubHeader(headers, 'x-ratelimit-resource')

  if (limit == null && remaining == null && used == null && reset == null && !resource) {
    return null
  }

  return {
    limit,
    remaining,
    used,
    reset,
    resource,
  }
}

export function isGithubRateLimitNearLimit(
  githubRateLimit: GithubRateLimitInfo,
  threshold = GITHUB_RATE_LIMIT_NEAR_THRESHOLD,
) {
  if (githubRateLimit.limit == null || githubRateLimit.remaining == null || githubRateLimit.limit <= 0) {
    return false
  }

  return githubRateLimit.remaining / githubRateLimit.limit < threshold
}

function logGithubRateLimit(route: string, status: number, headers: GithubResponseHeaders | undefined) {
  const githubRateLimit = extractGithubRateLimitInfo(headers)
  if (!githubRateLimit) {
    return
  }

  const isNearLimit = isGithubRateLimitNearLimit(githubRateLimit)

  if (isNearLimit) {
    const log = logger.warn.bind(logger)

    log({
      route,
      status,
      githubRateLimit,
    }, 'GitHub rate limit status')
  }
}

function recordGithubRequestMetric(
  route: string,
  status: number,
  headers: GithubResponseHeaders | undefined,
  durationMs: number,
  notModified = false,
) {
  const context = getGithubMetricsContext()
  const operation = context?.operation ?? route

  githubMetricsCollector.recordGithubApiEvent({
    userId: context?.userId,
    operation,
    scope: context?.scope,
    route,
    status,
    durationMs,
    notModified,
    rateLimit: extractGithubRateLimitInfo(headers),
  })
}

function buildGithubRequestOptions<Route extends keyof Endpoints>(
  token: string,
  params?: Endpoints[Route]['parameters'],
  headers?: Record<string, string>,
): GithubRequestOptions<Route> {
  return {
    ...(params ?? {}),
    headers: githubAuthHeaders(token, headers),
  } as GithubRequestOptions<Route>
}

async function requestGithubData<Route extends keyof Endpoints>(
  route: Route,
  {
    token,
    params,
    headers,
  }: {
    token: string
    params?: Endpoints[Route]['parameters']
    headers?: Record<string, string>
  },
): Promise<Endpoints[Route]['response']['data']> {
  const startedAt = Date.now()

  try {
    const response = await request(route, buildGithubRequestOptions<Route>(token, params, headers))
    const durationMs = Date.now() - startedAt
    logGithubRateLimit(route, response.status, response.headers)
    recordGithubRequestMetric(route, response.status, response.headers, durationMs)
    return response.data as Endpoints[Route]['response']['data']
  }
  catch (error) {
    const githubError = error as GithubErrorLike
    const durationMs = Date.now() - startedAt
    logGithubRateLimit(route, githubError.status ?? 0, githubError.response?.headers)
    recordGithubRequestMetric(route, githubError.status ?? 0, githubError.response?.headers, durationMs)
    throw error
  }
}

async function requestGithubWithoutData<Route extends keyof Endpoints>(
  route: Route,
  {
    token,
    params,
    headers,
  }: {
    token: string
    params?: Endpoints[Route]['parameters']
    headers?: Record<string, string>
  },
): Promise<void> {
  const startedAt = Date.now()

  try {
    const response = await request(route, buildGithubRequestOptions<Route>(token, params, headers))
    const durationMs = Date.now() - startedAt
    logGithubRateLimit(route, response.status, response.headers)
    recordGithubRequestMetric(route, response.status, response.headers, durationMs)
  }
  catch (error) {
    const githubError = error as GithubErrorLike
    const durationMs = Date.now() - startedAt
    logGithubRateLimit(route, githubError.status ?? 0, githubError.response?.headers)
    recordGithubRequestMetric(route, githubError.status ?? 0, githubError.response?.headers, durationMs)
    throw error
  }
}

async function requestGithubConditionally<Route extends keyof Endpoints>(
  route: Route,
  { token, params, etag, lastModified, headers }: GithubConditionalRequestOptions<Route>,
): Promise<GithubConditionalResponse<Route>> {
  const conditionalHeaders: Record<string, string> = {}
  const startedAt = Date.now()

  if (etag) {
    conditionalHeaders['if-none-match'] = etag
  }

  if (lastModified) {
    conditionalHeaders['if-modified-since'] = lastModified
  }

  try {
    const response = await request(route, buildGithubRequestOptions<Route>(
      token,
      params,
      {
        ...headers,
        ...conditionalHeaders,
      },
    ))
    const durationMs = Date.now() - startedAt
    logGithubRateLimit(route, response.status, response.headers)
    recordGithubRequestMetric(route, response.status, response.headers, durationMs)

    return {
      data: response.data as Endpoints[Route]['response']['data'] | null,
      notModified: false,
      etag: readGithubHeader(response.headers, 'etag'),
      lastModified: readGithubHeader(response.headers, 'last-modified'),
    }
  }
  catch (error) {
    const githubError = error as GithubErrorLike
    const durationMs = Date.now() - startedAt
    logGithubRateLimit(route, githubError.status ?? 0, githubError.response?.headers)
    if (githubError.status === 304) {
      recordGithubRequestMetric(route, 304, githubError.response?.headers, durationMs, true)
      return {
        data: null,
        notModified: true,
        etag: readGithubHeader(githubError.response?.headers, 'etag'),
        lastModified: readGithubHeader(githubError.response?.headers, 'last-modified'),
      }
    }

    recordGithubRequestMetric(route, githubError.status ?? 0, githubError.response?.headers, durationMs)
    throw error
  }
}

export async function fetchGithubNotifications(
  { token, params }:
  { token: string, params: NotificationsParams },
): Promise<NotificationResponse[]> {
  return requestGithubData('GET /notifications', {
    token,
    params,
  })
}

export async function fetchGithubPullRequests(
  { token, params }:
  { token: string, params: ListPullsParams },
): Promise<PullRequestResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/pulls', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestsConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/pulls'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/pulls'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/pulls'>(
    'GET /repos/{owner}/{repo}/pulls',
    options,
  )
}

export async function fetchGithubSearchIssues(
  { token, params }:
  { token: string, params: SearchIssuesParams },
): Promise<SearchIssuesResponse> {
  return requestGithubData('GET /search/issues', {
    token,
    params,
  })
}

export async function fetchGithubUserRepositories(
  { token, params }:
  { token: string, params: UserRepositoriesParams },
): Promise<UserRepositoryResponse[]> {
  return requestGithubData('GET /user/repos', {
    token,
    params,
  })
}

export async function fetchGithubPullRequest(
  { token, params }:
  { token: string, params: PullRequestParams },
): Promise<PullRequestDetailsResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/pulls/{pull_number}', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/pulls/{pull_number}'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/pulls/{pull_number}'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/pulls/{pull_number}'>(
    'GET /repos/{owner}/{repo}/pulls/{pull_number}',
    options,
  )
}

export async function patchGithubPullRequest(
  { token, params }:
  { token: string, params: UpdatePullRequestParams },
): Promise<UpdatePullRequestResponse> {
  return requestGithubData('PATCH /repos/{owner}/{repo}/pulls/{pull_number}', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestCommitsPage(
  { token, params }:
  { token: string, params: PullRequestCommitsParams },
): Promise<PullRequestCommitResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/pulls/{pull_number}/commits', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestCommitsAllPages(
  { token, params, perPage = 100}: {
    token: string
    params: Omit<PullRequestCommitsParams, 'per_page' | 'page'>
    perPage?: number
  },
): Promise<PullRequestCommitResponse[]> {
  const commits: PullRequestCommitResponse[] = []
  let page = 1

  while (true) {
    const data = await fetchGithubPullRequestCommitsPage({
      token,
      params: {
        ...params,
        per_page: perPage,
        page,
      },
    })
    commits.push(...data)

    if (data.length < perPage) {
      break
    }
    page += 1
  }

  return commits
}

export async function compareGithubRefs(
  { token, params }:
  { token: string, params: CompareParams },
) {
  return requestGithubData('GET /repos/{owner}/{repo}/compare/{basehead}', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestFilesPage(
  { token, params }:
  { token: string, params: PullRequestFilesParams },
): Promise<PullRequestFileResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/pulls/{pull_number}/files', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestFilesAllPages(
  { token, params, perPage = 100}: {
    token: string
    params: Omit<PullRequestFilesParams, 'per_page' | 'page'>
    perPage?: number
  },
): Promise<PullRequestFileResponse[]> {
  const files: PullRequestFileResponse[] = []
  let page = 1

  while (true) {
    const data = await fetchGithubPullRequestFilesPage({
      token,
      params: {
        ...params,
        per_page: perPage,
        page,
      },
    })
    files.push(...data)

    if (data.length < perPage) {
      break
    }
    page += 1
  }

  return files
}

export async function fetchGithubCommit(
  { token, params }:
  { token: string, params: CommitParams },
): Promise<CommitResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/commits/{ref}', {
    token,
    params,
  })
}

export async function fetchGithubCommitFilesAllPages(
  { token, params, perPage = 100}: {
    token: string
    params: Omit<CommitParams, 'per_page' | 'page'>
    perPage?: number
  },
): Promise<CommitFileResponse[]> {
  const files: CommitFileResponse[] = []
  let page = 1

  while (true) {
    const commit = await fetchGithubCommit({
      token,
      params: {
        ...params,
        per_page: perPage,
        page,
      },
    })
    const pageFiles = commit.files ?? []
    files.push(...pageFiles)

    if (pageFiles.length < perPage) {
      break
    }
    page += 1
  }

  return files
}

export async function fetchGithubPullRequestComments(
  { token, params }: { token: string, params: PullRequestCommentsParams },
): Promise<PullRequestCommentResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestReviews(
  { token, params }: { token: string, params: PullRequestReviewsParams },
): Promise<PullRequestReviewResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestReviewsConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews'>(
    'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews',
    options,
  )
}

export async function createGithubPullRequestComment(
  { token, params }: { token: string, params: CreatePullRequestCommentParams },
): Promise<CreatePullRequestCommentResponse> {
  return requestGithubData('POST /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
    token,
    params,
  })
}

export async function createGithubPullRequestReview(
  { token, params }: { token: string, params: CreatePullRequestReviewParams },
): Promise<CreatePullRequestReviewResponse> {
  return requestGithubData('POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews', {
    token,
    params,
  })
}

export async function createGithubPullRequestCommentReply(
  { token, params }: { token: string, params: CreatePullRequestCommentReplyParams },
): Promise<CreatePullRequestCommentReplyResponse> {
  return requestGithubData('POST /repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies', {
    token,
    params,
  })
}

export async function patchGithubPullRequestComment(
  { token, params }: { token: string, params: UpdatePullRequestCommentParams },
): Promise<UpdatePullRequestCommentResponse> {
  return requestGithubData('PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}', {
    token,
    params,
  })
}

export async function deleteGithubPullRequestComment(
  { token, params}: { token: string, params: DeletePullRequestCommentParams },
): Promise<void> {
  await requestGithubWithoutData('DELETE /repos/{owner}/{repo}/pulls/comments/{comment_id}', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryContent(
  { token, params}: { token: string, params: GetContentParams },
): Promise<GetContentResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/contents/{path}', {
    token,
    params,
    headers: {
      accept: 'application/vnd.github.raw+json',
    },
  })
}

export async function fetchGithubRepositoryContentConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/contents/{path}'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/contents/{path}'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/contents/{path}'>(
    'GET /repos/{owner}/{repo}/contents/{path}',
    {
      ...options,
      headers: {
        accept: 'application/vnd.github.raw+json',
        ...options.headers,
      },
    },
  )
}

export async function fetchGithubViewer({ token }: { token: string }): Promise<GithubUserResponse> {
  return requestGithubData('GET /user', {
    token,
  })
}

export async function fetchGithubRepository(
  { token, params }:
  { token: string, params: GithubRepositoryParameters },
): Promise<GithubRepositoryResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}'>(
    'GET /repos/{owner}/{repo}',
    options,
  )
}

export async function fetchGithubRepositoryIssues(
  { token, params }:
  { token: string, params: GithubIssueParameters },
): Promise<GithubIssueResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/issues', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryIssuesConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/issues'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/issues'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/issues'>(
    'GET /repos/{owner}/{repo}/issues',
    options,
  )
}

export async function fetchGithubRepositoryIssue(
  { token, params }:
  { token: string, params: GithubIssueDetailsParameters },
): Promise<GithubIssueDetailsResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/issues/{issue_number}', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryIssueConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/issues/{issue_number}'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/issues/{issue_number}'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/issues/{issue_number}'>(
    'GET /repos/{owner}/{repo}/issues/{issue_number}',
    options,
  )
}

export async function patchGithubIssue(
  { token, params }:
  { token: string, params: UpdateIssueParams },
): Promise<UpdateIssueResponse> {
  return requestGithubData('PATCH /repos/{owner}/{repo}/issues/{issue_number}', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryIssueComments(
  { token, params }:
  { token: string, params: GithubIssueDetailsCommentParameters },
): Promise<GithubIssueDetailsCommentResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/issues/{issue_number}/comments', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryIssueCommentsConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/issues/{issue_number}/comments'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/issues/{issue_number}/comments'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/issues/{issue_number}/comments'>(
    'GET /repos/{owner}/{repo}/issues/{issue_number}/comments',
    options,
  )
}

export async function createGithubIssueComment(
  { token, params }:
  { token: string, params: CreateIssueCommentParams },
): Promise<CreateIssueCommentResponse> {
  return requestGithubData('POST /repos/{owner}/{repo}/issues/{issue_number}/comments', {
    token,
    params,
  })
}

export async function patchGithubIssueComment(
  { token, params }:
  { token: string, params: UpdateIssueCommentParams },
): Promise<UpdateIssueCommentResponse> {
  return requestGithubData('PATCH /repos/{owner}/{repo}/issues/comments/{comment_id}', {
    token,
    params,
  })
}

export async function deleteGithubIssueComment(
  { token, params }:
  { token: string, params: DeleteIssueCommentParams },
): Promise<void> {
  await requestGithubWithoutData('DELETE /repos/{owner}/{repo}/issues/comments/{comment_id}', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryTrees(
  { token, params }:
  { token: string, params: GithubRepositoryTreeParams },
): Promise<GithubRepositoryTreesResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/git/trees/{tree_sha}', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryTreesConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/git/trees/{tree_sha}'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/git/trees/{tree_sha}'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/git/trees/{tree_sha}'>(
    'GET /repos/{owner}/{repo}/git/trees/{tree_sha}',
    options,
  )
}

export async function fetchGithubRepositoryBranches(
  { token, params }:
  { token: string, params: GithubRepositoryBranchesParameters },
): Promise<GithubRepositoryBranchesResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/branches', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryBranchesConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/branches'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/branches'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/branches'>(
    'GET /repos/{owner}/{repo}/branches',
    options,
  )
}

export async function fetchGithubRepositoryReadme(
  { token, params }:
  { token: string, params: GithubRepositoryReadmeParameters },
): Promise<GithubRepositoryReadmeResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/readme', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryReadmeConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/readme'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/readme'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/readme'>(
    'GET /repos/{owner}/{repo}/readme',
    options,
  )
}
