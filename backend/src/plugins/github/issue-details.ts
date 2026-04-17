import type { GithubCacheValidator, GithubCacheValidators } from './cache/github-cache.js'
import type {
  GithubIssueDetails,
  GithubIssueDetailsCommentResponse,
  GithubIssueDetailsResponse,
} from './types.js'
import { formatGithubUser, mapGithubIssueComment } from './formatter.js'

export const GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY = 'issue'
export const GITHUB_ISSUE_DETAILS_COMMENTS_VALIDATOR_KEY = 'issueComments'

function buildGithubIssueDetailsPayload(
  {
    owner,
    repo,
    issue,
    issueComments,
  }: {
    owner: string
    repo: string
    issue: GithubIssueDetailsResponse
    issueComments: GithubIssueDetailsCommentResponse[]
  },
): GithubIssueDetails {
  return {
    node_id: issue.node_id,
    reactions: [],
    id: issue.id,
    number: issue.number,
    title: issue.title,
    state: issue.state,
    state_reason: issue.state_reason,
    created_at: issue.created_at,
    updated_at: issue.updated_at,
    closed_at: issue.closed_at,
    labels: issue.labels,
    body: issue.body,
    comments: issueComments.map(mapGithubIssueComment),
    user: formatGithubUser(issue.user),
    repository: {
      owner,
      repo,
    },
  } satisfies GithubIssueDetails
}

export function buildGithubIssueDetailsValidators(
  {
    issue,
    issueComments,
  }: {
    issue?: GithubCacheValidator
    issueComments?: GithubCacheValidator
  },
): GithubCacheValidators {
  return {
    [GITHUB_ISSUE_DETAILS_ISSUE_VALIDATOR_KEY]: issue ?? {},
    [GITHUB_ISSUE_DETAILS_COMMENTS_VALIDATOR_KEY]: issueComments ?? {},
  }
}

export function mergeGithubIssueDetailsPayload(
  {
    owner,
    repo,
    cachedPayload,
    issue,
    issueComments,
  }: {
    owner: string
    repo: string
    cachedPayload: GithubIssueDetails | null
    issue: GithubIssueDetailsResponse | null
    issueComments: GithubIssueDetailsCommentResponse[] | null
  },
): GithubIssueDetails {
  if (issue) {
    const nextPayload = buildGithubIssueDetailsPayload({
      owner,
      repo,
      issue,
      issueComments: issueComments ?? [],
    })

    if (issueComments) {
      return nextPayload
    }

    return {
      ...nextPayload,
      comments: cachedPayload?.comments ?? [],
    }
  }

  if (!cachedPayload) {
    throw new Error('Cannot merge GitHub issue details without either a fresh issue payload or a cached payload')
  }

  if (!issueComments) {
    return cachedPayload
  }

  return {
    ...cachedPayload,
    comments: issueComments.map(mapGithubIssueComment),
  }
}
