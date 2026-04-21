import type {
  CreatePullRequestResponse,
  GithubCommitAuthorIdentity,
  GithubGraphqlPullRequestCommitNode,
  GithubGraphqlPullRequestNode,
  GithubGraphqlPullRequestResult,
  GithubIssueCommentResponseSource,
  GithubIssueDescriptionUpdate,
  GithubIssueDetailsComment,
  GithubLabel,
  GithubPullRequest,
  GithubPullRequestAuthor,
  GithubPullRequestCommit,
  GithubPullRequestCommitUser,
  GithubPullRequestDescriptionUpdate,
  GithubPullRequestFile,
  GithubPullRequestFileSource,
  GithubPullRequestReview,
  GithubPullRequestReviewComment,
  GithubPullRequestReviewResponseSource,
  GithubReviewCommentResponse,
  PullRequestResponse,
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

function mapGithubGraphqlPullRequestState(
  state: GithubGraphqlPullRequestNode['state'],
): PullRequestResponse['state'] {
  return state === 'OPEN' ? 'open' : 'closed'
}

function mapGithubLabel(
  label: { name?: string | null, color?: string | null } | null | undefined,
): GithubLabel | null {
  const name = label?.name?.trim()

  if (!name) {
    return null
  }

  const color = label?.color?.trim()

  return {
    name,
    ...(color ? { color } : {}),
  }
}

export function mapGithubGraphqlPullRequest(
  pullRequest: GithubGraphqlPullRequestNode,
): GithubGraphqlPullRequestResult {
  const labels = (pullRequest.labels?.nodes ?? [])
    .flatMap((label) => {
      const mappedLabel = mapGithubLabel(label)
      return mappedLabel ? [mappedLabel] : []
    })
  const reviewsCount = pullRequest.reviews?.totalCount ?? 0

  return {
    pullRequest: {
      number: pullRequest.number,
      title: pullRequest.title,
      state: mapGithubGraphqlPullRequestState(pullRequest.state),
      draft: pullRequest.isDraft,
      created_at: pullRequest.createdAt,
      closed_at: pullRequest.closedAt,
      merged_at: pullRequest.mergedAt,
      updated_at: pullRequest.updatedAt,
      comments_count: (pullRequest.comments?.totalCount ?? 0) + reviewsCount,
      author: mapGithubPullRequestAuthor({
        login: pullRequest.author?.login ?? undefined,
        avatar_url: pullRequest.author?.avatarUrl ?? null,
        type: pullRequest.author?.__typename ?? null,
      }),
      labels,
      repository: {
        owner: pullRequest.repository.owner.login,
        repo: pullRequest.repository.name,
      },
    },
  }
}

export function mapGithubPullRequest(
  pullRequest: PullRequestResponse | CreatePullRequestResponse,
): GithubPullRequest {
  const labels = pullRequest.labels
    .flatMap((label) => {
      if (typeof label !== 'object' || !label) {
        return []
      }

      const mappedLabel = mapGithubLabel(label)
      return mappedLabel ? [mappedLabel] : []
    })

  return {
    number: pullRequest.number,
    title: pullRequest.title,
    state: pullRequest.state,
    draft: Boolean(pullRequest.draft),
    created_at: pullRequest.created_at,
    closed_at: pullRequest.closed_at,
    merged_at: pullRequest.merged_at,
    updated_at: pullRequest.updated_at,
    comments_count: 0,
    author: mapGithubPullRequestAuthor(pullRequest.user),
    labels,
    repository: {
      owner: pullRequest.base.repo.owner.login,
      repo: pullRequest.base.repo.name,
    },
  }
}

export function mapGithubPullRequestAuthor<
  U extends { login?: string | null, avatar_url?: string | null, type?: string | null },
>(user: U | null | undefined): GithubPullRequestAuthor {
  const login = user?.login?.trim() || 'unknown'

  return {
    login,
    avatar_url: user?.avatar_url ?? null,
    is_bot: user?.type === 'Bot' || login.endsWith('[bot]'),
  }
}

export function mapGithubPullRequestReviewComment(
  comment: GithubReviewCommentResponse,
): GithubPullRequestReviewComment {
  return {
    node_id: comment.node_id,
    reactions: [],
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
    node_id: review.node_id,
    reactions: [],
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

export function mapGithubIssueComment(
  comment: GithubIssueCommentResponseSource,
): GithubIssueDetailsComment {
  return {
    node_id: comment.node_id,
    reactions: [],
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

function normalizeNullableText(value: string | null | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed || null
}

function commitAuthorDedupeKeys(author: GithubCommitAuthorIdentity): string[] {
  const keys: string[] = []
  if (author.login) {
    keys.push(`login:${author.login.toLowerCase()}`)
  }
  if (author.email) {
    keys.push(`email:${author.email.toLowerCase()}`)
  }
  if (keys.length === 0 && author.name) {
    keys.push(`name:${author.name.toLowerCase()}`)
  }
  return keys
}

function pushUniqueCommitAuthor(
  authors: GithubCommitAuthorIdentity[],
  seen: Set<string>,
  author: GithubCommitAuthorIdentity,
) {
  if (!author.name && !author.email && !author.login) {
    return
  }

  const keys = commitAuthorDedupeKeys(author)
  if (keys.some(key => seen.has(key))) {
    return
  }

  for (const key of keys) {
    seen.add(key)
  }
  authors.push(author)
}

function mapGithubGraphqlPullRequestCommitUser(
  user: { login?: string | null, avatarUrl?: string | null } | null | undefined,
): GithubPullRequestCommitUser | null {
  const login = normalizeNullableText(user?.login)
  if (!login) {
    return null
  }

  return {
    login,
    avatar_url: normalizeNullableText(user?.avatarUrl),
  }
}

export function mapGithubGraphqlCommitAuthors(
  commitAuthors: Array<{
    name?: string | null
    email?: string | null
    user?: {
      login?: string | null
      avatarUrl?: string | null
    } | null
  }>,
): GithubCommitAuthorIdentity[] {
  const authors: GithubCommitAuthorIdentity[] = []
  const seen = new Set<string>()

  for (const author of commitAuthors) {
    const login = normalizeNullableText(author.user?.login)
    pushUniqueCommitAuthor(authors, seen, {
      name: normalizeNullableText(author.name) ?? login,
      email: normalizeNullableText(author.email),
      login,
      avatar_url: normalizeNullableText(author.user?.avatarUrl),
    })
  }

  return authors
}

export function mapGithubGraphqlPullRequestCommit(
  node: GithubGraphqlPullRequestCommitNode,
): GithubPullRequestCommit {
  const commit = node.commit
  const parent = (commit.parents.nodes ?? []).find(parent => parent?.oid)

  return {
    sha: commit.oid,
    message: commit.message,
    authored_at: commit.authoredDate,
    committed_at: commit.committedDate,
    parent_sha: parent?.oid ?? null,
    author: mapGithubGraphqlPullRequestCommitUser(commit.author?.user),
    committer: mapGithubGraphqlPullRequestCommitUser(commit.committer?.user),
    authors: mapGithubGraphqlCommitAuthors((commit.authors.nodes ?? []).filter(author => author !== null)),
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
