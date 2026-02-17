import type { Endpoints } from '@octokit/types'
import { request } from '@octokit/request'

export type ListPullsParams = Endpoints['GET /repos/{owner}/{repo}/pulls']['parameters']
export type CompareParams
  = Endpoints['GET /repos/{owner}/{repo}/compare/{basehead}']['parameters']
export type PullRequestParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['parameters']
export type PullRequestCommentsParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['parameters']
export type CreatePullRequestCommentParams
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments']['parameters']
export type CreatePullRequestCommentReplyParams
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies']['parameters']
export type UpdatePullRequestCommentParams
  = Endpoints['PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}']['parameters']
export type DeletePullRequestCommentParams
  = Endpoints['DELETE /repos/{owner}/{repo}/pulls/comments/{comment_id}']['parameters']
export type PullRequestFilesParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/files']['parameters']
export type NotificationsParams = Endpoints['GET /notifications']['parameters']
export type GetContentParams
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['parameters']

export type NotificationResponse = Endpoints['GET /notifications']['response']['data'][number]
export type PullRequestResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls']['response']['data'][number]
export type PullRequestDetailsResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['response']['data']
export type PullRequestCommentResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['response']['data'][number]
export type CreatePullRequestCommentResponse
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments']['response']['data']
export type CreatePullRequestCommentReplyResponse
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies']['response']['data']
export type UpdatePullRequestCommentResponse
  = Endpoints['PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}']['response']['data']
export type PullRequestFileResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/files']['response']['data'][number]
export type GetContentResponse
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['response']['data']
export type GithubUserResponse = Endpoints['GET /user']['response']['data']

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

export async function createGithubPullRequestCommentReply(
  token: string,
  params: CreatePullRequestCommentReplyParams,
): Promise<CreatePullRequestCommentReplyResponse> {
  const { data } = await request('POST /repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies', {
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

export async function deleteGithubPullRequestComment(
  token: string,
  params: DeletePullRequestCommentParams,
): Promise<void> {
  await request('DELETE /repos/{owner}/{repo}/pulls/comments/{comment_id}', {
    ...params,
    headers: githubAuthHeaders(token),
  })
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
