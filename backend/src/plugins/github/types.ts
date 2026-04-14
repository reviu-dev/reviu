import type { Endpoints } from '@octokit/types'

export type ListPullsParams = Endpoints['GET /repos/{owner}/{repo}/pulls']['parameters']
export type CompareParams
  = Endpoints['GET /repos/{owner}/{repo}/compare/{basehead}']['parameters']
export type PullRequestParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['parameters']
export type PullRequestCommitsParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/commits']['parameters']
export type PullRequestCommentsParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['parameters']
export type PullRequestReviewsParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews']['parameters']
export type UpdatePullRequestParams
  = Endpoints['PATCH /repos/{owner}/{repo}/pulls/{pull_number}']['parameters']
export type CreatePullRequestParams
  = Endpoints['POST /repos/{owner}/{repo}/pulls']['parameters']
export type MergePullRequestParams
  = Endpoints['PUT /repos/{owner}/{repo}/pulls/{pull_number}/merge']['parameters']
export type UpdatePullRequestBranchParams
  = Endpoints['PUT /repos/{owner}/{repo}/pulls/{pull_number}/update-branch']['parameters']
export type CreatePullRequestCommentParams
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments']['parameters']
export type CreatePullRequestReviewParams
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews']['parameters']
export type CreatePullRequestCommentReplyParams
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies']['parameters']
export type UpdatePullRequestCommentParams
  = Endpoints['PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}']['parameters']
export type DeletePullRequestCommentParams
  = Endpoints['DELETE /repos/{owner}/{repo}/pulls/comments/{comment_id}']['parameters']
export type PullRequestFilesParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/files']['parameters']
export type CommitParams
  = Endpoints['GET /repos/{owner}/{repo}/commits/{ref}']['parameters']
export type CommitCheckRunsParams
  = Endpoints['GET /repos/{owner}/{repo}/commits/{ref}/check-runs']['parameters']
export type CommitCombinedStatusParams
  = Endpoints['GET /repos/{owner}/{repo}/commits/{ref}/status']['parameters']
export type WorkflowRunsParams
  = Endpoints['GET /repos/{owner}/{repo}/actions/runs']['parameters']
export type WorkflowRunJobsParams
  = Endpoints['GET /repos/{owner}/{repo}/actions/runs/{run_id}/jobs']['parameters']
export type BranchRulesParams
  = Endpoints['GET /repos/{owner}/{repo}/rules/branches/{branch}']['parameters']
export type UserRepositoriesParams = Endpoints['GET /user/repos']['parameters']
export type NotificationsParams = Endpoints['GET /notifications']['parameters']
export type GetContentParams
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['parameters']
export type RepositoryLabelsParams
  = Endpoints['GET /repos/{owner}/{repo}/labels']['parameters']
export type RepositoryAssigneesParams
  = Endpoints['GET /repos/{owner}/{repo}/assignees']['parameters']
export type AddIssueAssigneesParams
  = Endpoints['POST /repos/{owner}/{repo}/issues/{issue_number}/assignees']['parameters']
export type RemoveIssueAssigneesParams
  = Endpoints['DELETE /repos/{owner}/{repo}/issues/{issue_number}/assignees']['parameters']
export type AddIssueLabelsParams
  = Endpoints['POST /repos/{owner}/{repo}/issues/{issue_number}/labels']['parameters']
export type RemoveIssueLabelParams
  = Endpoints['DELETE /repos/{owner}/{repo}/issues/{issue_number}/labels/{name}']['parameters']
export type RequestPullRequestReviewersParams
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/requested_reviewers']['parameters']
export type RemovePullRequestReviewersParams
  = Endpoints['DELETE /repos/{owner}/{repo}/pulls/{pull_number}/requested_reviewers']['parameters']

export type NotificationResponse = Endpoints['GET /notifications']['response']['data'][number]
export type PullRequestResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls']['response']['data'][number]
export type PullRequestDetailsResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['response']['data']
export type PullRequestCommitResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/commits']['response']['data'][number]
export type PullRequestCommentResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['response']['data'][number]
export type PullRequestReviewResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews']['response']['data'][number]
export type UpdatePullRequestResponse
  = Endpoints['PATCH /repos/{owner}/{repo}/pulls/{pull_number}']['response']['data']
export type CreatePullRequestResponse
  = Endpoints['POST /repos/{owner}/{repo}/pulls']['response']['data']
export type MergePullRequestResponse
  = Endpoints['PUT /repos/{owner}/{repo}/pulls/{pull_number}/merge']['response']['data']
export type CreatePullRequestCommentResponse
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments']['response']['data']
export type CreatePullRequestReviewResponse
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews']['response']['data']
export type CreatePullRequestCommentReplyResponse
  = Endpoints['POST /repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies']['response']['data']
export type UpdatePullRequestCommentResponse
  = Endpoints['PATCH /repos/{owner}/{repo}/pulls/comments/{comment_id}']['response']['data']
export type PullRequestFileResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/files']['response']['data'][number]
export type CommitResponse = Endpoints['GET /repos/{owner}/{repo}/commits/{ref}']['response']['data']
export type CommitFileResponse = NonNullable<CommitResponse['files']>[number]
export type CommitCheckRunsResponse
  = Endpoints['GET /repos/{owner}/{repo}/commits/{ref}/check-runs']['response']['data']
export type CommitCombinedStatusResponse
  = Endpoints['GET /repos/{owner}/{repo}/commits/{ref}/status']['response']['data']
export type WorkflowRunsResponse
  = Endpoints['GET /repos/{owner}/{repo}/actions/runs']['response']['data']
export type WorkflowRunJobsResponse
  = Endpoints['GET /repos/{owner}/{repo}/actions/runs/{run_id}/jobs']['response']['data']
export type BranchRulesResponse
  = Endpoints['GET /repos/{owner}/{repo}/rules/branches/{branch}']['response']['data']
export type UserRepositoryResponse = Endpoints['GET /user/repos']['response']['data'][number]
export type RepositoryLabelResponse
  = Endpoints['GET /repos/{owner}/{repo}/labels']['response']['data'][number]
export type RepositoryAssigneeResponse
  = Endpoints['GET /repos/{owner}/{repo}/assignees']['response']['data'][number]
export type GithubUserResponse = Endpoints['GET /user']['response']['data']
export type GithubIssueResponse = Endpoints['GET /repos/{owner}/{repo}/issues']['response']['data'][number]
export type GithubIssueDetailsResponse = Endpoints['GET /repos/{owner}/{repo}/issues/{issue_number}']['response']['data']
export type GithubIssueDetailsParameters = Endpoints['GET /repos/{owner}/{repo}/issues/{issue_number}']['parameters']
export type UpdateIssueParams = Endpoints['PATCH /repos/{owner}/{repo}/issues/{issue_number}']['parameters']
export type UpdateIssueResponse = Endpoints['PATCH /repos/{owner}/{repo}/issues/{issue_number}']['response']['data']
export type GithubIssueDetailsCommentResponse = Endpoints['GET /repos/{owner}/{repo}/issues/{issue_number}/comments']['response']['data'][number]
export type GithubIssueDetailsCommentParameters = Endpoints['GET /repos/{owner}/{repo}/issues/{issue_number}/comments']['parameters']
export type CreateIssueCommentParams = Endpoints['POST /repos/{owner}/{repo}/issues/{issue_number}/comments']['parameters']
export type CreateIssueCommentResponse = Endpoints['POST /repos/{owner}/{repo}/issues/{issue_number}/comments']['response']['data']
export type UpdateIssueCommentParams = Endpoints['PATCH /repos/{owner}/{repo}/issues/comments/{comment_id}']['parameters']
export type UpdateIssueCommentResponse = Endpoints['PATCH /repos/{owner}/{repo}/issues/comments/{comment_id}']['response']['data']
export type DeleteIssueCommentParams = Endpoints['DELETE /repos/{owner}/{repo}/issues/comments/{comment_id}']['parameters']
export type GithubRepositoryResponse = Endpoints['GET /repos/{owner}/{repo}']['response']['data']
export type GithubRepositoryParameters = Endpoints['GET /repos/{owner}/{repo}']['parameters']
export type GithubRepositoryTreesResponse = Endpoints['GET /repos/{owner}/{repo}/git/trees/{tree_sha}']['response']['data']
export type GithubRepositoryTreeParams = Endpoints['GET /repos/{owner}/{repo}/git/trees/{tree_sha}']['parameters']
export type GithubRepositoryBranchesResponse = Endpoints['GET /repos/{owner}/{repo}/branches']['response']['data'][number]
export type GithubRepositoryBranchesParameters = Endpoints['GET /repos/{owner}/{repo}/branches']['parameters']
export type GithubRepositoryReadmeParameters = Endpoints['GET /repos/{owner}/{repo}/readme']['parameters']

export interface GithubRepository {
  owner: string
  repo: string
}

export interface GithubPullRequestAuthor {
  login: NonNullable<PullRequestResponse['user']>['login']
  avatar_url: NonNullable<PullRequestResponse['user']>['avatar_url'] | null
  is_bot: boolean
}

export interface GithubLabel {
  name: string
  color?: string | null
}

export interface GithubGraphqlPullRequestActor {
  __typename?: string | null
  login?: string | null
  avatarUrl?: string | null
}

export interface GithubGraphqlPullRequestNode {
  number: number
  title: string
  state: 'OPEN' | 'CLOSED' | 'MERGED'
  isDraft: boolean
  createdAt: string
  updatedAt: string
  closedAt: string | null
  mergedAt: string | null
  author: GithubGraphqlPullRequestActor | null
  labels?: {
    nodes?: Array<{
      name?: string | null
      color?: string | null
    } | null> | null
  } | null
  repository: {
    owner: {
      login: string
    }
    name: string
  }
  comments: {
    totalCount: number
  }
  reviews?: {
    totalCount: number
  } | null
}

export interface GithubGraphqlPullRequestResult {
  pullRequest: GithubPullRequest
}

export interface GithubPullRequest {
  number: PullRequestResponse['number']
  title: PullRequestResponse['title']
  state: PullRequestResponse['state']
  draft: NonNullable<PullRequestResponse['draft']>
  created_at: PullRequestResponse['created_at']
  closed_at: PullRequestResponse['closed_at']
  merged_at: PullRequestResponse['merged_at']
  updated_at: PullRequestResponse['updated_at']
  comments_count: number
  author: GithubPullRequestAuthor
  labels: GithubLabel[]
  repository: GithubRepository
}

export type GithubPullRequestSearchReviewStatus
  = 'any'
    | 'none'
    | 'required'
    | 'approved'
    | 'changes_requested'

export type GithubPullRequestSearchSort
  = 'updated_desc'
    | 'created_desc'
    | 'created_asc'
    | 'comments_desc'

export interface GithubPullRequestSearchFilters {
  repos: string[]
  labels: string[]
  authors: string[]
  assignees: string[]
  requested_reviewers: string[]
  review_status: GithubPullRequestSearchReviewStatus
  include_drafts: boolean
  base: string | null
  sort: GithubPullRequestSearchSort
}

export type GithubIssueSearchSort = 'updated_desc' | 'created_desc' | 'created_asc' | 'comments_desc'

export interface GithubIssueSearchFilters {
  repos: string[]
  labels: string[]
  authors: string[]
  assignees: string[]
  sort: GithubIssueSearchSort
}

export interface GithubPullRequestFilterOptionUser {
  login: string
  avatar_url: string | null
}

export interface GithubPullRequestFilterOptions {
  labels: Array<{
    name: string
  }>
  authors: GithubPullRequestFilterOptionUser[]
  assignees: GithubPullRequestFilterOptionUser[]
}

export interface GithubPullRequestReviewCommentUser {
  login: PullRequestCommentResponse['user']['login']
  avatar_url: PullRequestCommentResponse['user']['avatar_url']
}

export interface GithubPullRequestReviewComment {
  id: PullRequestCommentResponse['id']
  pull_request_review_id: PullRequestCommentResponse['pull_request_review_id']
  diff_hunk: PullRequestCommentResponse['diff_hunk']
  path: PullRequestCommentResponse['path']
  position: PullRequestCommentResponse['position']
  original_position: PullRequestCommentResponse['original_position']
  commit_id: PullRequestCommentResponse['commit_id']
  original_commit_id: PullRequestCommentResponse['original_commit_id']
  in_reply_to_id: PullRequestCommentResponse['in_reply_to_id']
  user: GithubPullRequestReviewCommentUser
  body: PullRequestCommentResponse['body']
  created_at: PullRequestCommentResponse['created_at']
  updated_at: PullRequestCommentResponse['updated_at']
  start_line: PullRequestCommentResponse['start_line']
  original_start_line: PullRequestCommentResponse['original_start_line']
  start_side: PullRequestCommentResponse['start_side']
  line: PullRequestCommentResponse['line']
  original_line: PullRequestCommentResponse['original_line']
  side: PullRequestCommentResponse['side']
}

export interface GithubPullRequestReviewUser {
  login: NonNullable<PullRequestReviewResponse['user']>['login']
  avatar_url: NonNullable<PullRequestReviewResponse['user']>['avatar_url']
}

export interface GithubPullRequestReview {
  id: PullRequestReviewResponse['id']
  body: PullRequestReviewResponse['body']
  state: PullRequestReviewResponse['state']
  submitted_at: PullRequestReviewResponse['submitted_at']
  commit_id: PullRequestReviewResponse['commit_id']
  html_url: PullRequestReviewResponse['html_url']
  user: GithubPullRequestReviewUser | null
}

export interface GithubPullRequestIssueCommentUser {
  login: NonNullable<GithubIssueDetailsCommentResponse['user']>['login']
  avatar_url: NonNullable<GithubIssueDetailsCommentResponse['user']>['avatar_url']
}

export interface GithubPullRequestIssueComment {
  id: GithubIssueDetailsCommentResponse['id']
  body: GithubIssueDetailsCommentResponse['body']
  created_at: GithubIssueDetailsCommentResponse['created_at']
  updated_at: GithubIssueDetailsCommentResponse['updated_at']
  user: GithubPullRequestIssueCommentUser | null
}

export interface GithubPullRequestDetails {
  node_id: PullRequestDetailsResponse['node_id']
  number: PullRequestDetailsResponse['number']
  title: PullRequestDetailsResponse['title']
  state: PullRequestDetailsResponse['state']
  draft: NonNullable<PullRequestDetailsResponse['draft']>
  created_at: PullRequestDetailsResponse['created_at']
  updated_at: PullRequestDetailsResponse['updated_at']
  merged_at: PullRequestDetailsResponse['merged_at']
  merge_base_sha: string
  base_sha: PullRequestDetailsResponse['base']['sha']
  head_sha: PullRequestDetailsResponse['head']['sha']
  base_ref_name: PullRequestDetailsResponse['base']['ref']
  head_ref_name: PullRequestDetailsResponse['head']['ref']
  body: PullRequestDetailsResponse['body']
  author: GithubPullRequestAuthor
  assignees: GithubPullRequestFilterOptionUser[]
  requested_reviewers: GithubPullRequestFilterOptionUser[]
  comments: PullRequestDetailsResponse['comments']
  review_comments: PullRequestDetailsResponse['review_comments']
  commits: PullRequestDetailsResponse['commits']
  additions: PullRequestDetailsResponse['additions']
  deletions: PullRequestDetailsResponse['deletions']
  changed_files: PullRequestDetailsResponse['changed_files']
  labels: PullRequestDetailsResponse['labels']
  repository: GithubRepository
  head_repository: GithubRepository
}

export type GithubPullRequestMergeMethod = NonNullable<MergePullRequestParams['merge_method']>

export type GithubPullRequestMergeReadinessStatus
  = | 'checking'
    | 'ready'
    | 'blocked'
    | 'forbidden'
    | 'draft'
    | 'closed'
    | 'merged'

export interface GithubPullRequestMergeReadiness {
  status: GithubPullRequestMergeReadinessStatus
  message: string
  current_head_sha: PullRequestDetailsResponse['head']['sha']
  available_methods: GithubPullRequestMergeMethod[]
  default_method: GithubPullRequestMergeMethod | null
  can_merge_now: boolean
  viewer_can_merge: boolean
  mergeable_state: PullRequestDetailsResponse['mergeable_state'] | null
  rebaseable: PullRequestDetailsResponse['rebaseable']
  auto_merge_enabled: boolean
}

export interface GithubPullRequestMergeResult {
  merged: MergePullRequestResponse['merged']
  sha: MergePullRequestResponse['sha']
  message: MergePullRequestResponse['message']
  method: GithubPullRequestMergeMethod
}

export type GithubPullRequestChecksRollupState = 'success' | 'pending' | 'failure'

export interface GithubPullRequestWorkflowStep {
  number: number
  name: string
  status: string | null
  conclusion: string | null
  state: GithubPullRequestChecksRollupState
  started_at: string | null
  completed_at: string | null
}

export interface GithubPullRequestWorkflowJob {
  id: number
  name: string
  status: string | null
  conclusion: string | null
  state: GithubPullRequestChecksRollupState
  started_at: string | null
  completed_at: string | null
  html_url: string | null
  required: boolean
  app_name: string | null
  app_slug: string | null
  app_avatar_url: string | null
  steps: GithubPullRequestWorkflowStep[]
}

export interface GithubPullRequestWorkflowRun {
  id: number
  name: string | null
  display_title: string | null
  event: string
  status: string | null
  conclusion: string | null
  state: GithubPullRequestChecksRollupState
  created_at: string
  updated_at: string
  run_started_at: string | null
  run_number: number
  run_attempt: number | null
  html_url: string | null
  jobs: GithubPullRequestWorkflowJob[]
}

export interface GithubPullRequestCheckRun {
  id: number
  name: string
  status: string | null
  conclusion: string | null
  state: GithubPullRequestChecksRollupState
  started_at: string | null
  completed_at: string | null
  html_url: string | null
  details_url: string | null
  required: boolean
  app_name: string | null
  app_slug: string | null
  app_avatar_url: string | null
  title: string | null
  summary: string | null
  text: string | null
  annotations_count: number
}

export interface GithubPullRequestLegacyStatus {
  id: number
  context: string
  status: string
  state: GithubPullRequestChecksRollupState
  description: string | null
  target_url: string | null
  avatar_url: string | null
  created_at: string
  updated_at: string
  required: boolean
}

export interface GithubPullRequestChecksSummary {
  head_sha: PullRequestDetailsResponse['head']['sha']
  overall_state: GithubPullRequestChecksRollupState
  required_state: GithubPullRequestChecksRollupState
  total_checks: number
  successful_checks: number
  failed_checks: number
  pending_checks: number
  required_checks_total: number
  required_checks_passed: number
  required_checks_failed: number
  required_checks_pending: number
  required_contexts: string[]
  missing_required_contexts: string[]
  requires_up_to_date_branch: boolean
  actions_runs: GithubPullRequestWorkflowRun[]
  other_checks: GithubPullRequestCheckRun[]
  legacy_statuses: GithubPullRequestLegacyStatus[]
}

export interface GithubPullRequestDescriptionUpdate {
  number: UpdatePullRequestResponse['number']
  body: UpdatePullRequestResponse['body']
  updated_at: UpdatePullRequestResponse['updated_at']
}

export interface GithubPullRequestCommitUser {
  login: NonNullable<PullRequestCommitResponse['author']>['login']
  avatar_url: NonNullable<PullRequestCommitResponse['author']>['avatar_url']
}

export interface GithubPullRequestCommit {
  sha: PullRequestCommitResponse['sha']
  message: PullRequestCommitResponse['commit']['message']
  authored_at: NonNullable<PullRequestCommitResponse['commit']['author']>['date'] | null
  committed_at: NonNullable<PullRequestCommitResponse['commit']['committer']>['date'] | null
  parent_sha: PullRequestCommitResponse['parents'][number]['sha'] | null
  author: GithubPullRequestCommitUser | null
  committer: GithubPullRequestCommitUser | null
}

export interface GithubPullRequestFile {
  filename: PullRequestFileResponse['filename']
  status: PullRequestFileResponse['status']
  patch: PullRequestFileResponse['patch']
  previous_filename: PullRequestFileResponse['previous_filename']
}

export interface GithubNotificationRepositoryOwner {
  login: NotificationResponse['repository']['owner']['login']
  avatar_url: NotificationResponse['repository']['owner']['avatar_url']
}

export interface GithubNotificationRepository {
  name: NotificationResponse['repository']['name']
  full_name: NotificationResponse['repository']['full_name']
  owner: GithubNotificationRepositoryOwner
}

export interface GithubNotificationSubject {
  title: NotificationResponse['subject']['title']
  type: NotificationResponse['subject']['type']
  url: NotificationResponse['subject']['url']
  latest_comment_url: NotificationResponse['subject']['latest_comment_url']
}

export interface GithubNotification {
  id: NotificationResponse['id']
  repository: GithubNotificationRepository
  subject: GithubNotificationSubject
  reason: NotificationResponse['reason']
  unread: NotificationResponse['unread']
  updated_at: NotificationResponse['updated_at']
  last_read_at: NotificationResponse['last_read_at']
  url: NotificationResponse['url']
  subscription_url: NotificationResponse['subscription_url']
}

export interface GithubIssue {
  id: GithubIssueResponse['id']
  number: GithubIssueResponse['number']
  title: GithubIssueResponse['title']
  state: GithubIssueResponse['state']
  state_reason: GithubIssueResponse['state_reason']
  created_at: GithubIssueResponse['created_at']
  updated_at: GithubIssueResponse['updated_at']
  closed_at: GithubIssueResponse['closed_at'] | null
  labels: GithubIssueResponse['labels']
  comments_count: GithubIssueResponse['comments']
  user: {
    login: NonNullable<GithubIssueResponse['user']>['login']
    name?: NonNullable<GithubIssueResponse['user']>['name']
    avatar_url: NonNullable<GithubIssueResponse['user']>['avatar_url']
  } | null
  repository: GithubRepository
}

export interface GithubIssueDetailsComment {
  id: GithubIssueDetailsCommentResponse['id']
  body: GithubIssueDetailsCommentResponse['body']
  created_at: GithubIssueDetailsCommentResponse['created_at']
  updated_at: GithubIssueDetailsCommentResponse['updated_at']
  user: {
    login: NonNullable<GithubIssueDetailsCommentResponse['user']>['login']
    name?: NonNullable<GithubIssueDetailsCommentResponse['user']>['name']
    avatar_url: NonNullable<GithubIssueDetailsCommentResponse['user']>['avatar_url']
  } | null
}

export interface GithubIssueDetails {
  id: GithubIssueResponse['id']
  number: GithubIssueResponse['number']
  title: GithubIssueResponse['title']
  body: GithubIssueResponse['body']
  state: GithubIssueResponse['state']
  state_reason: GithubIssueResponse['state_reason']
  created_at: GithubIssueResponse['created_at']
  updated_at: GithubIssueResponse['updated_at']
  closed_at: GithubIssueResponse['closed_at'] | null
  labels: GithubIssueResponse['labels']
  comments: GithubIssueDetailsComment[]
  user: {
    login: NonNullable<GithubIssueResponse['user']>['login']
    name?: NonNullable<GithubIssueResponse['user']>['name']
    avatar_url: NonNullable<GithubIssueResponse['user']>['avatar_url']
  } | null
  repository: GithubRepository
}

export interface GithubIssueDescriptionUpdate {
  id: UpdateIssueResponse['id']
  number: UpdateIssueResponse['number']
  body: UpdateIssueResponse['body']
  updated_at: UpdateIssueResponse['updated_at']
}

export interface GithubRepositoryDetails {
  name: GithubRepositoryResponse['name']
  full_name: GithubRepositoryResponse['full_name']
  private: GithubRepositoryResponse['private']
  description: GithubRepositoryResponse['description']
  homepage: GithubRepositoryResponse['homepage']
  language: GithubRepositoryResponse['language']
  default_branch: GithubRepositoryResponse['default_branch']
  stargazers_count: GithubRepositoryResponse['stargazers_count']
  forks_count: GithubRepositoryResponse['forks_count']
  subscribers_count: GithubRepositoryResponse['subscribers_count']
  open_issues_count: GithubRepositoryResponse['open_issues_count']
  size: GithubRepositoryResponse['size']
  pushed_at: GithubRepositoryResponse['pushed_at']
  html_url: GithubRepositoryResponse['html_url']
  owner: {
    login: GithubRepositoryResponse['owner']['login']
    name?: GithubRepositoryResponse['owner']['name']
    avatar_url: GithubRepositoryResponse['owner']['avatar_url']
  }
  license: {
    key: NonNullable<GithubRepositoryResponse['license']>['key']
    name: NonNullable<GithubRepositoryResponse['license']>['name']
    spdx_id: NonNullable<GithubRepositoryResponse['license']>['spdx_id']
  } | null
}

export interface GithubUserRepository {
  owner: string
  repo: string
  full_name: UserRepositoryResponse['full_name']
  description: UserRepositoryResponse['description']
  private: UserRepositoryResponse['private']
  owner_avatar_url: UserRepositoryResponse['owner']['avatar_url'] | null
  updated_at: NonNullable<UserRepositoryResponse['updated_at']>
}

export interface GithubRepositoryReadme {
  content: string | null
  path: string | null
}

export interface GithubRepositoryTree {
  sha: GithubRepositoryTreesResponse['sha']
  url?: GithubRepositoryTreesResponse['url']
  truncated: GithubRepositoryTreesResponse['truncated']
  tree: GithubRepositoryTreesResponse['tree']
}

export interface GithubRepositoryBranch {
  name: GithubRepositoryBranchesResponse['name']
  commit: {
    sha: GithubRepositoryBranchesResponse['commit']['sha']
    url: GithubRepositoryBranchesResponse['commit']['url']
  }
  protected: GithubRepositoryBranchesResponse['protected']
}

export interface GithubFileContent {
  content: string | null
}

export interface GithubFileAsset {
  contentBase64: string | null
}

export interface GithubPullRequestFileSource {
  filename: string
  status: string
  patch?: string | null
  previous_filename?: string | null
}

export type GithubReviewCommentResponse
  = | PullRequestCommentResponse
    | CreatePullRequestCommentResponse
    | CreatePullRequestCommentReplyResponse
    | UpdatePullRequestCommentResponse

export type GithubIssueCommentResponseSource
  = | GithubIssueDetailsCommentResponse
    | CreateIssueCommentResponse
    | UpdateIssueCommentResponse

export type GithubPullRequestReviewResponseSource
  = | PullRequestReviewResponse
    | CreatePullRequestReviewResponse
