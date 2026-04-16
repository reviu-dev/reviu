import type { Endpoints, RequestHeaders, RequestParameters } from '@octokit/types'
import type {
  AddIssueAssigneesParams,
  AddIssueLabelsParams,
  BranchRulesParams,
  BranchRulesResponse,
  CommitCheckRunsParams,
  CommitCheckRunsResponse,
  CommitCombinedStatusParams,
  CommitCombinedStatusResponse,
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
  CreatePullRequestParams,
  CreatePullRequestResponse,
  CreatePullRequestReviewParams,
  CreatePullRequestReviewResponse,
  DeleteIssueCommentParams,
  DeletePullRequestCommentParams,
  GithubGraphqlPullRequestNode,
  GithubIssue,
  GithubIssueDetailsCommentParameters,
  GithubIssueDetailsCommentResponse,
  GithubRepositoryParameters,
  GithubRepositoryResponse,
  GithubUserResponse,
  ListPullsParams,
  MergePullRequestParams,
  MergePullRequestResponse,
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
  RemoveIssueAssigneesParams,
  RemoveIssueLabelParams,
  RemovePullRequestReviewersParams,
  RepositoryAssigneeResponse,
  RepositoryAssigneesParams,
  RepositoryLabelResponse,
  RepositoryLabelsParams,
  RequestPullRequestReviewersParams,
  UpdateIssueCommentParams,
  UpdateIssueCommentResponse,
  UpdateIssueParams,
  UpdateIssueResponse,
  UpdatePullRequestBranchParams,
  UpdatePullRequestCommentParams,
  UpdatePullRequestCommentResponse,
  UpdatePullRequestParams,
  UpdatePullRequestResponse,
  UserRepositoriesParams,
  UserRepositoryResponse,
  WorkflowRunJobsParams,
  WorkflowRunJobsResponse,
  WorkflowRunsParams,
  WorkflowRunsResponse,
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

interface GithubConditionalRequestOptions<Route extends keyof Endpoints> {
  token: string
  params: Endpoints[Route]['parameters']
  etag?: string
  lastModified?: string
  headers?: Record<string, string>
}

interface GithubConditionalResponse<Route extends keyof Endpoints> {
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

interface GithubPaginatedCollectionResult<T> {
  items: T[]
  pageCount: number
  itemCount: number
  truncated: boolean
}

const GITHUB_RATE_LIMIT_NEAR_THRESHOLD = 0.1
const GITHUB_PAGINATED_COLLECTION_MAX_PAGES = 10
const GITHUB_PAGINATED_COLLECTION_MAX_ITEMS = 1000
const GITHUB_GRAPHQL_ROUTE = 'POST /graphql'

const GITHUB_GRAPHQL_PULL_REQUEST_LIST_FIELDS = `
  number
  title
  state
  isDraft
  createdAt
  updatedAt
  closedAt
  mergedAt
  author {
    __typename
    login
    avatarUrl
  }
  labels(first: 20) {
    nodes {
      name
      color
    }
  }
  repository {
    owner {
      login
    }
    name
  }
  comments {
    totalCount
  }
  reviews(states: [APPROVED, CHANGES_REQUESTED, COMMENTED, DISMISSED]) {
    totalCount
  }
`

const GITHUB_GRAPHQL_SEARCH_PULL_REQUESTS_QUERY = `
  query SearchPullRequests($query: String!, $first: Int!) {
    search(query: $query, type: ISSUE, first: $first) {
      issueCount
      nodes {
        ... on PullRequest {
          ${GITHUB_GRAPHQL_PULL_REQUEST_LIST_FIELDS}
        }
      }
    }
  }
`

const GITHUB_GRAPHQL_SEARCH_ISSUES_QUERY = `
  query SearchIssues($query: String!, $first: Int!) {
    search(query: $query, type: ISSUE, first: $first) {
      issueCount
      nodes {
        ... on Issue {
          number
          title
          state
          stateReason
          createdAt
          updatedAt
          closedAt
          comments {
            totalCount
          }
          author {
            login
            avatarUrl
          }
          labels(first: 10) {
            nodes {
              name
              color
            }
          }
          repository {
            owner {
              login
            }
            name
          }
        }
      }
    }
  }
`

const GITHUB_GRAPHQL_REPOSITORY_OVERVIEW_QUERY = `
  query RepositoryOverview($owner: String!, $name: String!) {
    repository(owner: $owner, name: $name) {
      id
      name
      nameWithOwner
      isPrivate
      viewerHasStarred
      description
      homepageUrl
      defaultBranchRef {
        name
        target {
          ... on Commit {
            oid
            message
            committedDate
            author {
              user {
                login
                avatarUrl
              }
            }
            history(first: 6) {
              nodes {
                oid
                message
                committedDate
                author {
                  user {
                    login
                    avatarUrl
                  }
                }
              }
            }
          }
        }
      }
      primaryLanguage {
        name
      }
      stargazerCount
      forkCount
      watchers {
        totalCount
      }
      diskUsage
      pushedAt
      url
      owner {
        login
        avatarUrl
      }
      licenseInfo {
        key
        name
        spdxId
      }
      languages(first: 20, orderBy: { field: SIZE, direction: DESC }) {
        totalSize
        edges {
          size
          node {
            name
            color
          }
        }
      }
    }
  }
`

const GITHUB_GRAPHQL_ADD_STAR_MUTATION = `
  mutation AddStar($starrableId: ID!) {
    addStar(input: { starrableId: $starrableId }) {
      starrable {
        viewerHasStarred
        ... on Repository {
          stargazerCount
        }
      }
    }
  }
`

const GITHUB_GRAPHQL_REMOVE_STAR_MUTATION = `
  mutation RemoveStar($starrableId: ID!) {
    removeStar(input: { starrableId: $starrableId }) {
      starrable {
        viewerHasStarred
        ... on Repository {
          stargazerCount
        }
      }
    }
  }
`

const GITHUB_GRAPHQL_MARK_PULL_REQUEST_READY_FOR_REVIEW_MUTATION = `
  mutation MarkPullRequestReadyForReview($pullRequestId: ID!) {
    markPullRequestReadyForReview(input: { pullRequestId: $pullRequestId }) {
      pullRequest {
        id
      }
    }
  }
`

const GITHUB_GRAPHQL_CONVERT_PULL_REQUEST_TO_DRAFT_MUTATION = `
  mutation ConvertPullRequestToDraft($pullRequestId: ID!) {
    convertPullRequestToDraft(input: { pullRequestId: $pullRequestId }) {
      pullRequest {
        id
      }
    }
  }
`

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

interface GithubGraphqlResponse<T> {
  data?: T
  errors?: Array<{
    message: string
  }>
}

interface GithubGraphqlIssueNode {
  number: number
  title: string
  state: string
  stateReason: string | null
  createdAt: string
  updatedAt: string
  closedAt: string | null
  comments: { totalCount: number }
  author: { login: string, avatarUrl: string } | null
  labels: { nodes: Array<{ name: string, color: string }> | null } | null
  repository: { owner: { login: string }, name: string }
}

interface GithubGraphqlSearchIssuesResponse {
  search: {
    issueCount: number
    nodes?: Array<GithubGraphqlIssueNode | null> | null
  }
}

interface GithubGraphqlSearchPullRequestsResponse {
  search: {
    issueCount: number
    nodes?: Array<GithubGraphqlPullRequestNode | null> | null
  }
}

interface GithubGraphqlMarkPullRequestReadyForReviewResponse {
  markPullRequestReadyForReview?: {
    pullRequest?: {
      id: string
    } | null
  } | null
}

interface GithubGraphqlConvertPullRequestToDraftResponse {
  convertPullRequestToDraft?: {
    pullRequest?: {
      id: string
    } | null
  } | null
}

interface GithubGraphqlRepositoryOverviewResponse {
  repository: {
    id: string
    name: string
    nameWithOwner: string
    isPrivate: boolean
    viewerHasStarred: boolean
    description: string | null
    homepageUrl: string | null
    defaultBranchRef: {
      name: string
      target: {
        oid: string
        message: string
        committedDate: string
        author: {
          user: {
            login: string
            avatarUrl: string
          } | null
        } | null
        history: {
          nodes: Array<{
            oid: string
            message: string
            committedDate: string
            author: {
              user: {
                login: string
                avatarUrl: string
              } | null
            } | null
          }>
        }
      } | null
    } | null
    primaryLanguage: { name: string } | null
    stargazerCount: number
    forkCount: number
    watchers: { totalCount: number }
    diskUsage: number | null
    pushedAt: string | null
    url: string
    owner: {
      login: string
      avatarUrl: string
    }
    licenseInfo: {
      key: string
      name: string
      spdxId: string | null
    } | null
    languages: {
      totalSize: number
      edges: Array<{
        size: number
        node: {
          name: string
          color: string | null
        }
      }>
    }
  }
}

interface GithubGraphqlStarMutationResponse {
  addStar?: {
    starrable: {
      viewerHasStarred: boolean
      stargazerCount: number
    }
  }
  removeStar?: {
    starrable: {
      viewerHasStarred: boolean
      stargazerCount: number
    }
  }
}

interface GithubStarResult {
  viewer_has_starred: boolean
  stargazers_count: number
}

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

function recordGithubPaginationMetric(
  pageCount: number,
  itemCount: number,
  truncated: boolean,
  durationMs: number,
) {
  const context = getGithubMetricsContext()

  if (!context?.operation || !context.scope) {
    return
  }

  githubMetricsCollector.recordPaginationEvent({
    userId: context.userId,
    operation: context.operation,
    scope: context.scope,
    pageCount,
    itemCount,
    truncated,
    durationMs,
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

async function requestGithubGraphqlData<T>(
  {
    token,
    query,
    variables,
  }: {
    token: string
    query: string
    variables?: Record<string, unknown>
  },
): Promise<T> {
  const startedAt = Date.now()

  try {
    const response = await request(GITHUB_GRAPHQL_ROUTE, {
      headers: githubAuthHeaders(token),
      query,
      variables,
    })
    const payload = response.data as GithubGraphqlResponse<T>
    if (payload.errors?.length) {
      throw Object.assign(new Error(payload.errors.map(error => error.message).join('; ')), {
        status: response.status,
        response: {
          headers: response.headers,
        },
      })
    }

    if (!payload.data) {
      throw Object.assign(new Error('GitHub GraphQL response is missing data'), {
        status: response.status,
        response: {
          headers: response.headers,
        },
      })
    }

    const durationMs = Date.now() - startedAt
    logGithubRateLimit(GITHUB_GRAPHQL_ROUTE, response.status, response.headers)
    recordGithubRequestMetric(GITHUB_GRAPHQL_ROUTE, response.status, response.headers, durationMs)

    return payload.data
  }
  catch (error) {
    const githubError = error as GithubErrorLike
    const durationMs = Date.now() - startedAt
    logGithubRateLimit(GITHUB_GRAPHQL_ROUTE, githubError.status ?? 0, githubError.response?.headers)
    recordGithubRequestMetric(
      GITHUB_GRAPHQL_ROUTE,
      githubError.status ?? 0,
      githubError.response?.headers,
      durationMs,
    )
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

async function fetchGithubCollectionAllPages<T, Params extends { per_page?: number, page?: number }>(
  fetchPage: (args: { token: string, params: Params }) => Promise<T[]>,
  {
    token,
    params,
    perPage = 100,
    maxPages = GITHUB_PAGINATED_COLLECTION_MAX_PAGES,
    maxItems = GITHUB_PAGINATED_COLLECTION_MAX_ITEMS,
    initialPageItems,
  }: {
    token: string
    params: Omit<Params, 'per_page' | 'page'>
    perPage?: number
    maxPages?: number
    maxItems?: number
    initialPageItems?: T[]
  },
): Promise<GithubPaginatedCollectionResult<T>> {
  const startedAt = Date.now()
  const items: T[] = []
  let pageCount = 0
  let truncated = false

  if (initialPageItems) {
    pageCount = 1

    if (initialPageItems.length > maxItems) {
      items.push(...initialPageItems.slice(0, maxItems))
      truncated = true
    }
    else {
      items.push(...initialPageItems)
    }

    if (initialPageItems.length < perPage || truncated) {
      recordGithubPaginationMetric(
        pageCount,
        items.length,
        truncated,
        Date.now() - startedAt,
      )

      return {
        items,
        pageCount,
        itemCount: items.length,
        truncated,
      }
    }
  }

  for (let page = initialPageItems ? 2 : 1; page <= maxPages; page += 1) {
    const pageItems = await fetchPage({
      token,
      params: {
        ...params,
        per_page: perPage,
        page,
      } as Params,
    })

    pageCount += 1

    const remainingCapacity = maxItems - items.length
    if (remainingCapacity <= 0) {
      truncated = true
      break
    }

    if (pageItems.length > remainingCapacity) {
      items.push(...pageItems.slice(0, remainingCapacity))
      truncated = true
      break
    }

    items.push(...pageItems)

    if (pageItems.length < perPage) {
      break
    }

    if (page === maxPages) {
      truncated = true
    }
  }

  if (truncated) {
    logger.warn({
      operation: getGithubMetricsContext()?.operation ?? null,
      pageCount,
      itemCount: items.length,
      maxItems,
      maxPages,
    }, 'GitHub paginated collection was truncated at configured limits')
  }

  recordGithubPaginationMetric(
    pageCount,
    items.length,
    truncated,
    Date.now() - startedAt,
  )

  return {
    items,
    pageCount,
    itemCount: items.length,
    truncated,
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

export async function markGithubNotificationDone(
  { token, threadId }: { token: string, threadId: string },
): Promise<void> {
  await requestGithubWithoutData('DELETE /notifications/threads/{thread_id}', {
    token,
    params: { thread_id: Number.parseInt(threadId) },
  })
}

export async function markGithubNotificationRead(
  { token, threadId }: { token: string, threadId: string },
): Promise<void> {
  await requestGithubWithoutData('PATCH /notifications/threads/{thread_id}', {
    token,
    params: { thread_id: Number.parseInt(threadId) },
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

export async function fetchGithubPullRequestSearchGraphql(
  {
    token,
    query,
    limit,
  }: {
    token: string
    query: string
    limit: number
  },
): Promise<{ nodes: GithubGraphqlPullRequestNode[], issueCount: number }> {
  const data = await requestGithubGraphqlData<GithubGraphqlSearchPullRequestsResponse>({
    token,
    query: GITHUB_GRAPHQL_SEARCH_PULL_REQUESTS_QUERY,
    variables: {
      query,
      first: limit,
    },
  })

  return {
    nodes: data.search.nodes?.flatMap(node => (node ? [node] : [])) ?? [],
    issueCount: data.search.issueCount,
  }
}

function mapGithubGraphqlIssue(node: GithubGraphqlIssueNode): GithubIssue {
  const stateReason = node.stateReason?.toLowerCase() ?? null
  return {
    id: node.number,
    number: node.number,
    title: node.title,
    state: node.state.toLowerCase(),
    state_reason: stateReason as GithubIssue['state_reason'],
    created_at: node.createdAt,
    updated_at: node.updatedAt,
    closed_at: node.closedAt,
    labels: (node.labels?.nodes ?? []).map(label => ({
      name: label.name,
      color: label.color,
    })),
    comments_count: node.comments.totalCount,
    user: node.author
      ? {
          login: node.author.login,
          avatar_url: node.author.avatarUrl,
        }
      : null,
    repository: {
      owner: node.repository.owner.login,
      repo: node.repository.name,
    },
  }
}

export async function fetchGithubIssueSearchGraphql(
  {
    token,
    query,
    limit,
  }: {
    token: string
    query: string
    limit: number
  },
): Promise<{ issues: GithubIssue[], issueCount: number }> {
  const data = await requestGithubGraphqlData<GithubGraphqlSearchIssuesResponse>({
    token,
    query: GITHUB_GRAPHQL_SEARCH_ISSUES_QUERY,
    variables: {
      query,
      first: limit,
    },
  })

  const issues = (data.search.nodes?.flatMap(node => (node ? [node] : [])) ?? [])
    .map(mapGithubGraphqlIssue)

  return { issues, issueCount: data.search.issueCount }
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

export async function fetchGithubRepositoryLabels(
  { token, params }:
  { token: string, params: RepositoryLabelsParams },
): Promise<RepositoryLabelResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/labels', {
    token,
    params,
  })
}

export async function fetchGithubRepositoryAssignees(
  { token, params }:
  { token: string, params: RepositoryAssigneesParams },
): Promise<RepositoryAssigneeResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/assignees', {
    token,
    params,
  })
}

export async function addGithubIssueLabels(
  { token, params }:
  { token: string, params: AddIssueLabelsParams },
): Promise<void> {
  await requestGithubData('POST /repos/{owner}/{repo}/issues/{issue_number}/labels', {
    token,
    params,
  })
}

export async function removeGithubIssueLabel(
  { token, params }:
  { token: string, params: RemoveIssueLabelParams },
): Promise<void> {
  await requestGithubData('DELETE /repos/{owner}/{repo}/issues/{issue_number}/labels/{name}', {
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

export async function createGithubPullRequest(
  { token, params }:
  { token: string, params: CreatePullRequestParams },
): Promise<CreatePullRequestResponse> {
  return requestGithubData('POST /repos/{owner}/{repo}/pulls', {
    token,
    params,
  })
}

export async function markGithubPullRequestReadyForReview(
  {
    token,
    pullRequestId,
  }: {
    token: string
    pullRequestId: string
  },
): Promise<void> {
  const data = await requestGithubGraphqlData<GithubGraphqlMarkPullRequestReadyForReviewResponse>({
    token,
    query: GITHUB_GRAPHQL_MARK_PULL_REQUEST_READY_FOR_REVIEW_MUTATION,
    variables: {
      pullRequestId,
    },
  })

  if (!data.markPullRequestReadyForReview?.pullRequest?.id) {
    throw new Error('GitHub GraphQL response is missing the updated pull request')
  }
}

export async function convertGithubPullRequestToDraft(
  {
    token,
    pullRequestId,
  }: {
    token: string
    pullRequestId: string
  },
): Promise<void> {
  const data = await requestGithubGraphqlData<GithubGraphqlConvertPullRequestToDraftResponse>({
    token,
    query: GITHUB_GRAPHQL_CONVERT_PULL_REQUEST_TO_DRAFT_MUTATION,
    variables: {
      pullRequestId,
    },
  })

  if (!data.convertPullRequestToDraft?.pullRequest?.id) {
    throw new Error('GitHub GraphQL response is missing the updated pull request')
  }
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

export async function addGithubIssueAssignees(
  { token, params }:
  { token: string, params: AddIssueAssigneesParams },
): Promise<void> {
  await requestGithubWithoutData('POST /repos/{owner}/{repo}/issues/{issue_number}/assignees', {
    token,
    params,
  })
}

export async function removeGithubIssueAssignees(
  { token, params }:
  { token: string, params: RemoveIssueAssigneesParams },
): Promise<void> {
  await requestGithubWithoutData('DELETE /repos/{owner}/{repo}/issues/{issue_number}/assignees', {
    token,
    params,
  })
}

export async function requestGithubPullRequestReviewers(
  { token, params }:
  { token: string, params: RequestPullRequestReviewersParams },
): Promise<void> {
  await requestGithubWithoutData('POST /repos/{owner}/{repo}/pulls/{pull_number}/requested_reviewers', {
    token,
    params,
  })
}

export async function removeGithubPullRequestReviewers(
  { token, params }:
  { token: string, params: RemovePullRequestReviewersParams },
): Promise<void> {
  await requestGithubWithoutData('DELETE /repos/{owner}/{repo}/pulls/{pull_number}/requested_reviewers', {
    token,
    params,
  })
}

export async function mergeGithubPullRequest(
  { token, params }:
  { token: string, params: MergePullRequestParams },
): Promise<MergePullRequestResponse> {
  return requestGithubData('PUT /repos/{owner}/{repo}/pulls/{pull_number}/merge', {
    token,
    params,
  })
}

export async function updateGithubPullRequestBranch(
  { token, params }:
  { token: string, params: UpdatePullRequestBranchParams },
): Promise<void> {
  await requestGithubWithoutData('PUT /repos/{owner}/{repo}/pulls/{pull_number}/update-branch', {
    token,
    params,
  })
}

async function fetchGithubPullRequestCommitsPage(
  { token, params }:
  { token: string, params: PullRequestCommitsParams },
): Promise<PullRequestCommitResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/pulls/{pull_number}/commits', {
    token,
    params,
  })
}

export async function fetchGithubCommitConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/commits/{ref}'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/commits/{ref}'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/commits/{ref}'>(
    'GET /repos/{owner}/{repo}/commits/{ref}',
    options,
  )
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

async function fetchGithubPullRequestFilesPage(
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

async function fetchGithubCommit(
  { token, params }:
  { token: string, params: CommitParams },
): Promise<CommitResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/commits/{ref}', {
    token,
    params,
  })
}

export async function fetchGithubCommitCheckRuns(
  { token, params }:
  { token: string, params: CommitCheckRunsParams },
): Promise<CommitCheckRunsResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/commits/{ref}/check-runs', {
    token,
    params,
  })
}

export async function fetchGithubCombinedStatusForRef(
  { token, params }:
  { token: string, params: CommitCombinedStatusParams },
): Promise<CommitCombinedStatusResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/commits/{ref}/status', {
    token,
    params,
  })
}

export async function fetchGithubWorkflowRuns(
  { token, params }:
  { token: string, params: WorkflowRunsParams },
): Promise<WorkflowRunsResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/actions/runs', {
    token,
    params,
  })
}

export async function fetchGithubWorkflowRunJobs(
  { token, params }:
  { token: string, params: WorkflowRunJobsParams },
): Promise<WorkflowRunJobsResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/actions/runs/{run_id}/jobs', {
    token,
    params,
  })
}

export async function fetchGithubBranchRules(
  { token, params }:
  { token: string, params: BranchRulesParams },
): Promise<BranchRulesResponse> {
  return requestGithubData('GET /repos/{owner}/{repo}/rules/branches/{branch}', {
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

async function fetchGithubPullRequestComments(
  { token, params }: { token: string, params: PullRequestCommentsParams },
): Promise<PullRequestCommentResponse[]> {
  return requestGithubData('GET /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
    token,
    params,
  })
}

export async function fetchGithubPullRequestCommentsConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/pulls/{pull_number}/comments'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/pulls/{pull_number}/comments'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/pulls/{pull_number}/comments'>(
    'GET /repos/{owner}/{repo}/pulls/{pull_number}/comments',
    options,
  )
}

export async function fetchGithubPullRequestCommentsAllPages(
  { token, params, perPage = 100, maxPages = GITHUB_PAGINATED_COLLECTION_MAX_PAGES, maxItems = GITHUB_PAGINATED_COLLECTION_MAX_ITEMS, initialPageItems }: {
    token: string
    params: Omit<PullRequestCommentsParams, 'per_page' | 'page'>
    perPage?: number
    maxPages?: number
    maxItems?: number
    initialPageItems?: PullRequestCommentResponse[]
  },
): Promise<GithubPaginatedCollectionResult<PullRequestCommentResponse>> {
  return fetchGithubCollectionAllPages(fetchGithubPullRequestComments, {
    token,
    params,
    perPage,
    maxPages,
    maxItems,
    initialPageItems,
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

export async function fetchGithubRepositoryContentObjectConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/contents/{path}'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/contents/{path}'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/contents/{path}'>(
    'GET /repos/{owner}/{repo}/contents/{path}',
    {
      ...options,
      headers: {
        accept: 'application/vnd.github.object+json',
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

export async function fetchGithubRepositoryOverview(
  { token, owner, name }: { token: string, owner: string, name: string },
): Promise<GithubGraphqlRepositoryOverviewResponse['repository']> {
  const data = await requestGithubGraphqlData<GithubGraphqlRepositoryOverviewResponse>({
    token,
    query: GITHUB_GRAPHQL_REPOSITORY_OVERVIEW_QUERY,
    variables: { owner, name },
  })

  return data.repository
}

export async function starGithubRepository(
  { token, repositoryId }: { token: string, repositoryId: string },
): Promise<GithubStarResult> {
  const data = await requestGithubGraphqlData<GithubGraphqlStarMutationResponse>({
    token,
    query: GITHUB_GRAPHQL_ADD_STAR_MUTATION,
    variables: { starrableId: repositoryId },
  })

  const starrable = data.addStar!.starrable
  return {
    viewer_has_starred: starrable.viewerHasStarred,
    stargazers_count: starrable.stargazerCount,
  }
}

export async function unstarGithubRepository(
  { token, repositoryId }: { token: string, repositoryId: string },
): Promise<GithubStarResult> {
  const data = await requestGithubGraphqlData<GithubGraphqlStarMutationResponse>({
    token,
    query: GITHUB_GRAPHQL_REMOVE_STAR_MUTATION,
    variables: { starrableId: repositoryId },
  })

  const starrable = data.removeStar!.starrable
  return {
    viewer_has_starred: starrable.viewerHasStarred,
    stargazers_count: starrable.stargazerCount,
  }
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

async function fetchGithubRepositoryIssueComments(
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

export async function fetchGithubRepositoryIssueCommentsAllPages(
  { token, params, perPage = 100, maxPages = GITHUB_PAGINATED_COLLECTION_MAX_PAGES, maxItems = GITHUB_PAGINATED_COLLECTION_MAX_ITEMS, initialPageItems }: {
    token: string
    params: Omit<GithubIssueDetailsCommentParameters, 'per_page' | 'page'>
    perPage?: number
    maxPages?: number
    maxItems?: number
    initialPageItems?: GithubIssueDetailsCommentResponse[]
  },
): Promise<GithubPaginatedCollectionResult<GithubIssueDetailsCommentResponse>> {
  return fetchGithubCollectionAllPages(fetchGithubRepositoryIssueComments, {
    token,
    params,
    perPage,
    maxPages,
    maxItems,
    initialPageItems,
  })
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

export async function fetchGithubRepositoryTreesConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/git/trees/{tree_sha}'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/git/trees/{tree_sha}'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/git/trees/{tree_sha}'>(
    'GET /repos/{owner}/{repo}/git/trees/{tree_sha}',
    options,
  )
}

export async function fetchGithubRepositoryCommitsConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/commits'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/commits'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/commits'>(
    'GET /repos/{owner}/{repo}/commits',
    options,
  )
}

export async function fetchGithubRepositoryBranchesConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/branches'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/branches'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/branches'>(
    'GET /repos/{owner}/{repo}/branches',
    options,
  )
}

export async function fetchGithubRepositoryReadmeConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/readme'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/readme'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/readme'>(
    'GET /repos/{owner}/{repo}/readme',
    options,
  )
}
