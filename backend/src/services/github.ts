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
export type SearchIssuesParams = Endpoints['GET /search/issues']['parameters']
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
export type SearchIssuesResponse = Endpoints['GET /search/issues']['response']['data']
export type SearchIssuesItemResponse = SearchIssuesResponse['items'][number]
export type GetContentResponse
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['response']['data']
export type GithubUserResponse = Endpoints['GET /user']['response']['data']
export type GithubIssueResponse = Endpoints['GET /repos/{owner}/{repo}/issues']['response']['data'][number]
export type GithubIssueParameters = Endpoints['GET /repos/{owner}/{repo}/issues']['parameters']
export type GithubIssueDetailsResponse = Endpoints['GET /repos/{owner}/{repo}/issues/{issue_number}']['response']['data']
export type GithubIssueDetailsParameters = Endpoints['GET /repos/{owner}/{repo}/issues/{issue_number}']['parameters']
export type GithubIssueDetailsCommentResponse = Endpoints['GET /repos/{owner}/{repo}/issues/{issue_number}/comments']['response']['data'][number]
export type GithubIssueDetailsCommentParameters = Endpoints['GET /repos/{owner}/{repo}/issues/{issue_number}/comments']['parameters']
export type GithubRepositoryResponse = Endpoints['GET /repos/{owner}/{repo}']['response']['data']
export type GithubRepositoryParameters = Endpoints['GET /repos/{owner}/{repo}']['parameters']
export type GithubRepositoryTreesResponse = Endpoints['GET /repos/{owner}/{repo}/git/trees/{tree_sha}']['response']['data']
export type GithubRepositoryTreeParams = Endpoints['GET /repos/{owner}/{repo}/git/trees/{tree_sha}']['parameters']
export type GithubRepositoryBranchesResponse = Endpoints['GET /repos/{owner}/{repo}/branches']['response']['data'][number]
export type GithubRepositoryBranchesParameters = Endpoints['GET /repos/{owner}/{repo}/branches']['parameters']
export type GithubRepositoryReadmeResponse = Endpoints['GET /repos/{owner}/{repo}/readme']['response']['data']
export type GithubRepositoryReadmeParameters = Endpoints['GET /repos/{owner}/{repo}/readme']['parameters']

function githubAuthHeaders(token: string, extraHeaders?: Record<string, string>) {
  return {
    authorization: `Bearer ${token}`,
    ...extraHeaders,
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

export async function fetchGithubPullRequestComments(
  { token, params }: { token: string, params: PullRequestCommentsParams },
): Promise<PullRequestCommentResponse[]> {
  const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
    ...params,
    headers: githubAuthHeaders(token),
  })
  return data
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
