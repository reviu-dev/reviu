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

interface GithubErrorLike {
  status?: number
  response?: {
    headers?: Record<string, string | number | string[] | undefined>
  }
}

function readGithubHeader(
  headers: Record<string, string | number | string[] | undefined> | undefined,
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

async function requestGithubConditionally<Route extends keyof Endpoints>(
  route: Route,
  { token, params, etag, lastModified, headers }: GithubConditionalRequestOptions<Route>,
): Promise<GithubConditionalResponse<Route>> {
  const conditionalHeaders: Record<string, string> = {}

  if (etag) {
    conditionalHeaders['if-none-match'] = etag
  }

  if (lastModified) {
    conditionalHeaders['if-modified-since'] = lastModified
  }

  try {
    const options = {
      ...params,
      headers: githubAuthHeaders(token, {
        ...headers,
        ...conditionalHeaders,
      }),
    } as Route extends keyof Endpoints ? Endpoints[Route]['parameters'] & RequestParameters : RequestParameters

    const response = await request(route, options)

    return {
      data: response.data as Endpoints[Route]['response']['data'] | null,
      notModified: false,
      etag: readGithubHeader(response.headers, 'etag'),
      lastModified: readGithubHeader(response.headers, 'last-modified'),
    }
  }
  catch (error) {
    const githubError = error as GithubErrorLike
    if (githubError.status === 304) {
      return {
        data: null,
        notModified: true,
        etag: readGithubHeader(githubError.response?.headers, 'etag'),
        lastModified: readGithubHeader(githubError.response?.headers, 'last-modified'),
      }
    }

    throw error
  }
}

export async function fetchGithubNotifications(
  { token, params }:
  { token: string, params: NotificationsParams },
): Promise<NotificationResponse[]> {
  const { data } = await request('GET /notifications', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequests(
  { token, params }:
  { token: string, params: ListPullsParams },
): Promise<PullRequestResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubSearchIssues(
  { token, params }:
  { token: string, params: SearchIssuesParams },
): Promise<SearchIssuesResponse> {
  const { data } = await request('GET /search/issues', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubUserRepositories(
  { token, params }:
  { token: string, params: UserRepositoriesParams },
): Promise<UserRepositoryResponse[]> {
  const { data } = await request('GET /user/repos', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequest(
  { token, params }:
  { token: string, params: PullRequestParams },
): Promise<PullRequestDetailsResponse> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
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
  const { data } = await request('PATCH /repos/{owner}/{repo}/pulls/{pull_number}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequestCommitsPage(
  { token, params }:
  { token: string, params: PullRequestCommitsParams },
): Promise<PullRequestCommitResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}/commits', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
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
  const { data } = await request('GET /repos/{owner}/{repo}/compare/{basehead}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequestFilesPage(
  { token, params }:
  { token: string, params: PullRequestFilesParams },
): Promise<PullRequestFileResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}/files', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
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
  const { data } = await request('GET /repos/{owner}/{repo}/commits/{ref}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
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
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequestReviews(
  { token, params }: { token: string, params: PullRequestReviewsParams },
): Promise<PullRequestReviewResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
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
  const { data } = await request('POST /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function createGithubPullRequestReview(
  { token, params }: { token: string, params: CreatePullRequestReviewParams },
): Promise<CreatePullRequestReviewResponse> {
  const { data } = await request('POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function createGithubPullRequestCommentReply(
  { token, params }: { token: string, params: CreatePullRequestCommentReplyParams },
): Promise<CreatePullRequestCommentReplyResponse> {
  const { data } = await request('POST /repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function patchGithubPullRequestComment(
  { token, params }: { token: string, params: UpdatePullRequestCommentParams },
): Promise<UpdatePullRequestCommentResponse> {
  const { data } = await request('PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function deleteGithubPullRequestComment(
  { token, params}: { token: string, params: DeletePullRequestCommentParams },
): Promise<void> {
  await request('DELETE /repos/{owner}/{repo}/pulls/comments/{comment_id}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
}

export async function fetchGithubRepositoryContent(
  { token, params}: { token: string, params: GetContentParams },
): Promise<GetContentResponse> {
  const { data } = await request('GET /repos/{owner}/{repo}/contents/{path}', {
    ...params,
    headers: githubAuthHeaders(token, {
      accept: 'application/vnd.github.raw+json',
    }),
  })
  return data
}

export async function fetchGithubViewer({ token }: { token: string }): Promise<GithubUserResponse> {
  const { data } = await request('GET /user', {
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubRepository(
  { token, params }:
  { token: string, params: GithubRepositoryParameters },
): Promise<GithubRepositoryResponse> {
  const { data } = await request('GET /repos/{owner}/{repo}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
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
  const { data } = await request('GET /repos/{owner}/{repo}/issues', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubRepositoryIssue(
  { token, params }:
  { token: string, params: GithubIssueDetailsParameters },
): Promise<GithubIssueDetailsResponse> {
  const { data } = await request('GET /repos/{owner}/{repo}/issues/{issue_number}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function patchGithubIssue(
  { token, params }:
  { token: string, params: UpdateIssueParams },
): Promise<UpdateIssueResponse> {
  const { data } = await request('PATCH /repos/{owner}/{repo}/issues/{issue_number}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubRepositoryIssueComments(
  { token, params }:
  { token: string, params: GithubIssueDetailsCommentParameters },
): Promise<GithubIssueDetailsCommentResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/issues/{issue_number}/comments', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function createGithubIssueComment(
  { token, params }:
  { token: string, params: CreateIssueCommentParams },
): Promise<CreateIssueCommentResponse> {
  const { data } = await request('POST /repos/{owner}/{repo}/issues/{issue_number}/comments', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function patchGithubIssueComment(
  { token, params }:
  { token: string, params: UpdateIssueCommentParams },
): Promise<UpdateIssueCommentResponse> {
  const { data } = await request('PATCH /repos/{owner}/{repo}/issues/comments/{comment_id}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function deleteGithubIssueComment(
  { token, params }:
  { token: string, params: DeleteIssueCommentParams },
): Promise<void> {
  await request('DELETE /repos/{owner}/{repo}/issues/comments/{comment_id}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
}

export async function fetchGithubRepositoryTrees(
  { token, params }:
  { token: string, params: GithubRepositoryTreeParams },
): Promise<GithubRepositoryTreesResponse> {
  const { data } = await request('GET /repos/{owner}/{repo}/git/trees/{tree_sha}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubRepositoryBranches(
  { token, params }:
  { token: string, params: GithubRepositoryBranchesParameters },
): Promise<GithubRepositoryBranchesResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/branches', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
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
  const { data } = await request('GET /repos/{owner}/{repo}/readme', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubRepositoryReadmeConditionally(
  options: GithubConditionalRequestOptions<'GET /repos/{owner}/{repo}/readme'>,
): Promise<GithubConditionalResponse<'GET /repos/{owner}/{repo}/readme'>> {
  return requestGithubConditionally<'GET /repos/{owner}/{repo}/readme'>(
    'GET /repos/{owner}/{repo}/readme',
    options,
  )
}
