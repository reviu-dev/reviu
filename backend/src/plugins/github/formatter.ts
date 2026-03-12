import type {
  GithubIssueCommentResponseSource,
  GithubIssueDescriptionUpdate,
  GithubIssueDetailsComment,
  GithubIssueDetailsCommentResponse,
  GithubPullRequest,
  GithubPullRequestCommit,
  GithubPullRequestCommitUser,
  GithubPullRequestDescriptionUpdate,
  GithubPullRequestFile,
  GithubPullRequestFileSource,
  GithubPullRequestIssueComment,
  GithubPullRequestReview,
  GithubPullRequestReviewComment,
  GithubPullRequestReviewResponseSource,
  GithubRepository,
  GithubReviewCommentResponse,
  PullRequestCommitResponse,
  PullRequestResponse,
  SearchIssuesItemResponse,
  UpdateIssueResponse,
  UpdatePullRequestResponse,
} from './types.js'

export function formatGithubUser<U extends { login: string, name?: string | null, avatar_url: string }>(user: U | null) {
  if (!user)
    return null

  return {
    login: user.login,
    name: user.name,
    avatar_url: user.avatar_url,
  }
}

export function mapGithubPullRequestReviewComment(
  comment: GithubReviewCommentResponse,
): GithubPullRequestReviewComment {
  return {
    id: comment.id,
    pull_request_review_id: comment.pull_request_review_id,
    diff_hunk: comment.diff_hunk,
    path: comment.path,
    position: comment.position,
    original_position: comment.original_position,
    commit_id: comment.commit_id,
    original_commit_id: comment.original_commit_id,
    in_reply_to_id: comment.in_reply_to_id,
    user: {
      login: comment.user.login,
      avatar_url: comment.user.avatar_url,
    },
    body: comment.body,
    created_at: comment.created_at,
    updated_at: comment.updated_at,
    start_line: comment.start_line,
    original_start_line: comment.original_start_line,
    start_side: comment.start_side,
    line: comment.line,
    original_line: comment.original_line,
    side: comment.side,
  }
}

export function mapGithubPullRequestReview(
  review: GithubPullRequestReviewResponseSource,
): GithubPullRequestReview {
  return {
    id: review.id,
    body: review.body,
    state: review.state,
    submitted_at: review.submitted_at,
    commit_id: review.commit_id,
    html_url: review.html_url,
    user: review.user
      ? {
          login: review.user.login,
          avatar_url: review.user.avatar_url,
        }
      : null,
  }
}

export function mapGithubPullRequestIssueComment(
  comment: GithubIssueDetailsCommentResponse,
): GithubPullRequestIssueComment {
  return {
    id: comment.id,
    body: comment.body,
    created_at: comment.created_at,
    updated_at: comment.updated_at,
    user: comment.user
      ? {
          login: comment.user.login,
          avatar_url: comment.user.avatar_url,
        }
      : null,
  }
}

export function mapGithubIssueComment(
  comment: GithubIssueCommentResponseSource,
): GithubIssueDetailsComment {
  return {
    id: comment.id,
    body: comment.body,
    created_at: comment.created_at,
    updated_at: comment.updated_at,
    user: formatGithubUser(comment.user),
  }
}

export function mapGithubPullRequestDescriptionUpdate(
  pullRequest: UpdatePullRequestResponse,
): GithubPullRequestDescriptionUpdate {
  return {
    number: pullRequest.number,
    body: pullRequest.body,
    updated_at: pullRequest.updated_at,
  }
}

export function mapGithubIssueDescriptionUpdate(
  issue: UpdateIssueResponse,
): GithubIssueDescriptionUpdate {
  return {
    id: issue.id,
    number: issue.number,
    body: issue.body,
    updated_at: issue.updated_at,
  }
}

export function mapGithubPullRequestCommitUser(
  user: PullRequestCommitResponse['author'] | PullRequestCommitResponse['committer'],
): GithubPullRequestCommitUser | null {
  if (!user) {
    return null
  }

  return {
    login: user.login,
    avatar_url: user.avatar_url,
  }
}

export function mapGithubPullRequestCommit(
  commit: PullRequestCommitResponse,
): GithubPullRequestCommit {
  return {
    sha: commit.sha,
    message: commit.commit.message,
    authored_at: commit.commit.author?.date ?? null,
    committed_at: commit.commit.committer?.date ?? null,
    parent_sha: commit.parents.at(0)?.sha ?? null,
    author: mapGithubPullRequestCommitUser(commit.author),
    committer: mapGithubPullRequestCommitUser(commit.committer),
  }
}

export function mapGithubPullRequestFile(file: GithubPullRequestFileSource): GithubPullRequestFile {
  return {
    filename: file.filename,
    status: file.status as GithubPullRequestFile['status'],
    patch: file.patch ?? undefined,
    previous_filename: file.previous_filename ?? undefined,
  }
}

function parseGithubRepositoryUrl(repositoryUrl: string): GithubRepository | null {
  try {
    const pathParts = new URL(repositoryUrl).pathname.split('/').filter(Boolean)
    if (pathParts.length < 3 || pathParts[0] !== 'repos') {
      return null
    }

    const owner = pathParts[1]
    const repo = pathParts[2]
    if (!owner || !repo) {
      return null
    }

    return { owner, repo }
  }
  catch {
    return null
  }
}

export function mapSearchIssueItemToPullRequest(item: SearchIssuesItemResponse): GithubPullRequest | null {
  const repository = parseGithubRepositoryUrl(item.repository_url)
  if (!repository || !item.pull_request) {
    return null
  }

  return {
    number: item.number,
    title: item.title,
    state: item.state as PullRequestResponse['state'],
    draft: Boolean(item.draft),
    merged_at: item.pull_request.merged_at ?? null,
    updated_at: item.updated_at,
    labels: item.labels
      .flatMap(label => (typeof label.name === 'string' && label.name.trim().length > 0 ? [{ name: label.name }] : [])),
    repository,
  }
}
