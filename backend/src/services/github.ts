import type { Endpoints } from '@octokit/types'
import { request } from '@octokit/request'

type ListPullsParams = Endpoints['GET /repos/{owner}/{repo}/pulls']['parameters']
type CompareParams
  = Endpoints['GET /repos/{owner}/{repo}/compare/{basehead}']['parameters']
type PullRequestParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['parameters']
type PullRequestCommentsParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['parameters']
type CreatePullRequestCommentParams
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments']['parameters']
type UpdatePullRequestCommentParams
  = Endpoints['PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}']['parameters']
type PullRequestFilesParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/files']['parameters']
type NotificationsParams = Endpoints['GET /notifications']['parameters']
type GetContentParams
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['parameters']

type NotificationResponse = Endpoints['GET /notifications']['response']['data'][number]
type PullRequestResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls']['response']['data'][number]
type PullRequestDetailsResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['response']['data']
type PullRequestCommentResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['response']['data'][number]
type CreatePullRequestCommentResponse
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments']['response']['data']
type UpdatePullRequestCommentResponse
  = Endpoints['PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}']['response']['data']
type PullRequestFileResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/files']['response']['data'][number]
type GetContentResponse
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['response']['data']
type GithubUserResponse = Endpoints['GET /user']['response']['data']

function githubAuthHeaders(token: string, extraHeaders?: Record<string, string>) {
  return {
    authorization: `Bearer ${token}`,
    ...extraHeaders,
  }
}

export async function fetchGithubNotifications(
  token: string,
  params: NotificationsParams,
): Promise<NotificationResponse[]> {
  const { data } = await request('GET /notifications', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequests(
  token: string,
  params: ListPullsParams,
): Promise<PullRequestResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequest(
  token: string,
  params: PullRequestParams,
): Promise<PullRequestDetailsResponse> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function compareGithubRefs(
  token: string,
  params: CompareParams,
) {
  const { data } = await request('GET /repos/{owner}/{repo}/compare/{basehead}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequestFilesPage(
  token: string,
  params: PullRequestFilesParams,
): Promise<PullRequestFileResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}/files', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubPullRequestFilesAllPages(
  token: string,
  params: Omit<PullRequestFilesParams, 'per_page' | 'page'>,
  perPage = 100,
): Promise<PullRequestFileResponse[]> {
  const files: PullRequestFileResponse[] = []
  let page = 1

  while (true) {
    const data = await fetchGithubPullRequestFilesPage(token, {
      ...params,
      per_page: perPage,
      page,
    })
    files.push(...data)

    if (data.length < perPage) {
      break
    }
    page += 1
  }

  return files
}

export async function fetchGithubPullRequestComments(
  token: string,
  params: PullRequestCommentsParams,
): Promise<PullRequestCommentResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function createGithubPullRequestComment(
  token: string,
  params: CreatePullRequestCommentParams,
): Promise<CreatePullRequestCommentResponse> {
  const { data } = await request('POST /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function patchGithubPullRequestComment(
  token: string,
  params: UpdatePullRequestCommentParams,
): Promise<UpdatePullRequestCommentResponse> {
  const { data } = await request('PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
}

export async function fetchGithubRepositoryContent(
  token: string,
  params: GetContentParams,
): Promise<GetContentResponse> {
  const { data } = await request('GET /repos/{owner}/{repo}/contents/{path}', {
    ...params,
    headers: githubAuthHeaders(token, {
      accept: 'application/vnd.github.raw+json',
    }),
  })
  return data
}

export async function fetchGithubViewer(token: string): Promise<GithubUserResponse> {
  const { data } = await request('GET /user', {
    headers: githubAuthHeaders(token),
  })
  return data
}
