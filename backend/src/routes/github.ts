import type { CompareParams, CreatePullRequestCommentParams, CreatePullRequestCommentReplyParams, CreatePullRequestCommentReplyResponse, CreatePullRequestCommentResponse, DeletePullRequestCommentParams, GetContentParams, GithubIssueDetailsCommentParameters, GithubIssueDetailsCommentResponse, GithubIssueDetailsParameters, GithubIssueParameters, GithubIssueResponse, GithubRepositoryBranchesParameters, GithubRepositoryBranchesResponse, GithubRepositoryParameters, GithubRepositoryReadmeParameters, GithubRepositoryResponse, GithubRepositoryTreeParams, GithubRepositoryTreesResponse, ListPullsParams, NotificationResponse, NotificationsParams, PullRequestCommentResponse, PullRequestCommentsParams, PullRequestDetailsResponse, PullRequestParams, PullRequestResponse, SearchIssuesItemResponse, SearchIssuesParams, UpdatePullRequestCommentParams, UpdatePullRequestCommentResponse, UserRepositoriesParams, UserRepositoryResponse } from '../services/github.js'
import { Buffer } from 'node:buffer'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import z from 'zod'
import { authMiddleware } from '../middlewares/auth.js'
import {
  compareGithubRefs,
  createGithubPullRequestComment,
  createGithubPullRequestCommentReply,
  deleteGithubPullRequestComment,
  fetchGithubNotifications,
  fetchGithubPullRequest,
  fetchGithubPullRequestComments,
  fetchGithubPullRequestFilesAllPages,
  fetchGithubPullRequests,
  fetchGithubRepository,
  fetchGithubRepositoryBranches,
  fetchGithubRepositoryContent,
  fetchGithubRepositoryIssue,
  fetchGithubRepositoryIssueComments,
  fetchGithubRepositoryIssues,
  fetchGithubRepositoryReadme,
  fetchGithubRepositoryTrees,
  fetchGithubSearchIssues,
  fetchGithubUserRepositories,
  patchGithubPullRequestComment,

} from '../services/github.js'

interface GithubRepository {
  owner: string
  repo: string
}

interface GithubPullRequest {
  number: PullRequestResponse['number']
  title: PullRequestResponse['title']
  state: PullRequestResponse['state']
  draft: NonNullable<PullRequestResponse['draft']>
  merged_at: PullRequestResponse['merged_at']
  updated_at: PullRequestResponse['updated_at']
  labels: { name: string }[]
  repository: GithubRepository
}

interface GithubPullRequestDetailsAuthor {
  login: PullRequestDetailsResponse['user']['login']
  avatar_url: PullRequestDetailsResponse['user']['avatar_url']
}

interface GithubPullRequestReviewCommentUser {
  login: PullRequestCommentResponse['user']['login']
  avatar_url: PullRequestCommentResponse['user']['avatar_url']
}

interface GithubPullRequestReviewComment {
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

interface GithubPullRequestDetails {
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
  author: GithubPullRequestDetailsAuthor
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

interface GithubNotificationRepositoryOwner {
  login: NotificationResponse['repository']['owner']['login']
  avatar_url: NotificationResponse['repository']['owner']['avatar_url']
}

interface GithubNotificationRepository {
  name: NotificationResponse['repository']['name']
  full_name: NotificationResponse['repository']['full_name']
  owner: GithubNotificationRepositoryOwner
}

interface GithubNotificationSubject {
  title: NotificationResponse['subject']['title']
  type: NotificationResponse['subject']['type']
  url: NotificationResponse['subject']['url']
  latest_comment_url: NotificationResponse['subject']['latest_comment_url']
}

interface GithubNotification {
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

interface GithubIssue {
  id: GithubIssueResponse['id']
  number: GithubIssueResponse['number']
  title: GithubIssueResponse['title']
  state: GithubIssueResponse['state']
  state_reason: GithubIssueResponse['state_reason']
  created_at: GithubIssueResponse['created_at']
  updated_at: GithubIssueResponse['updated_at']
  closed_at: GithubIssueResponse['closed_at'] | null
  labels: GithubIssueResponse['labels']
  user: {
    login: NonNullable<GithubIssueResponse['user']>['login']
    name?: NonNullable<GithubIssueResponse['user']>['name']
    avatar_url: NonNullable<GithubIssueResponse['user']>['avatar_url']
  } | null
  repository: GithubRepository
}

interface GithubIssueDetailsComment {
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

interface GithubIssueDetails {
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

interface GithubRepositoryDetails {
  name: GithubRepositoryResponse['name']
  full_name: GithubRepositoryResponse['full_name']
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

interface GithubUserRepository {
  owner: string
  repo: string
  full_name: UserRepositoryResponse['full_name']
  description: UserRepositoryResponse['description']
  private: UserRepositoryResponse['private']
  updated_at: NonNullable<UserRepositoryResponse['updated_at']>
}

interface GithubRepositoryReadme {
  content: string | null
  path: string | null
}

interface GithubRepositoryTree {
  sha: GithubRepositoryTreesResponse['sha']
  url?: GithubRepositoryTreesResponse['url']
  truncated: GithubRepositoryTreesResponse['truncated']
  tree: GithubRepositoryTreesResponse['tree']
}

interface GithubRepositoryBranch {
  name: GithubRepositoryBranchesResponse['name']
  commit: {
    sha: GithubRepositoryBranchesResponse['commit']['sha']
    url: GithubRepositoryBranchesResponse['commit']['url']
  }
  protected: GithubRepositoryBranchesResponse['protected']
}

interface GithubFileContent {
  content: string | null
}

type GithubReviewCommentResponse
  = | PullRequestCommentResponse
    | CreatePullRequestCommentResponse
    | CreatePullRequestCommentReplyResponse
    | UpdatePullRequestCommentResponse

const updatePullRequestCommentBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
})

const createPullRequestLineCommentBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
  path: z.string().trim().min(1, 'Missing comment path'),
  commitId: z.string().trim().min(1, 'Missing comment commit id'),
  line: z.number().int().positive(),
  side: z.enum(['LEFT', 'RIGHT']),
  startLine: z.number().int().positive().optional(),
  startSide: z.enum(['LEFT', 'RIGHT']).optional(),
})

const createPullRequestThreadReplyBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
})

function formatGithubUser<U extends { login: string, name?: string | null, avatar_url: string }>(user: U | null) {
  if (!user)
    return null

  return {
    login: user.login,
    name: user.name,
    avatar_url: user.avatar_url,
  }
}

function mapGithubPullRequestReviewComment(
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

const LATEST_PULL_REQUESTS_QUERY = 'author:@me is:pr is:open archived:false'
const NEED_REVIEWS_PULL_REQUESTS_QUERY = 'review-requested:@me is:pr is:open archived:false'
const LATEST_PULL_REQUESTS_LIMIT = 20
const LATEST_PULL_REQUESTS_CACHE_TTL_MS = 60_000

interface PullRequestSearchCacheEntry {
  pullRequests: GithubPullRequest[]
  expiresAt: number
}

const latestPullRequestsCache = new Map<string, PullRequestSearchCacheEntry>()
const latestPullRequestsInflight = new Map<string, Promise<GithubPullRequest[]>>()
const needReviewsPullRequestsCache = new Map<string, PullRequestSearchCacheEntry>()
const needReviewsPullRequestsInflight = new Map<string, Promise<GithubPullRequest[]>>()

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

function mapSearchIssueItemToPullRequest(item: SearchIssuesItemResponse): GithubPullRequest | null {
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

async function fetchPullRequestsSearchWithCache(
  cache: Map<string, PullRequestSearchCacheEntry>,
  inflight: Map<string, Promise<GithubPullRequest[]>>,
  cacheKey: string,
  githubToken: string,
  query: string,
) {
  const now = Date.now()
  const cachedEntry = cache.get(cacheKey)

  if (cachedEntry && cachedEntry.expiresAt > now) {
    return cachedEntry.pullRequests
  }

  const inflightRequest = inflight.get(cacheKey)
  if (inflightRequest) {
    try {
      return await inflightRequest
    }
    catch (error) {
      if (cachedEntry) {
        return cachedEntry.pullRequests
      }
      throw error
    }
  }

  const loadPullRequests = (async () => {
    const params: SearchIssuesParams = {
      q: query,
      sort: 'updated',
      order: 'desc',
      per_page: LATEST_PULL_REQUESTS_LIMIT,
    }

    const data = await fetchGithubSearchIssues({ token: githubToken, params })
    const pullRequests = data.items
      .flatMap((item) => {
        const pullRequest = mapSearchIssueItemToPullRequest(item)
        return pullRequest ? [pullRequest] : []
      })
      .slice(0, LATEST_PULL_REQUESTS_LIMIT)

    cache.set(cacheKey, {
      pullRequests,
      expiresAt: Date.now() + LATEST_PULL_REQUESTS_CACHE_TTL_MS,
    })

    return pullRequests
  })()

  inflight.set(cacheKey, loadPullRequests)

  try {
    return await loadPullRequests
  }
  catch (error) {
    if (cachedEntry) {
      return cachedEntry.pullRequests
    }
    throw error
  }
  finally {
    inflight.delete(cacheKey)
  }
}

const githubRouter = new Hono()

githubRouter.use('*', authMiddleware)

export const githubRoutes = githubRouter
  .get('/notifications', async (ctx) => {
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: NotificationsParams = {
        per_page: 50,
        all: false,
      }

      const data = await fetchGithubNotifications({ token: githubToken, params })

      const notifications: GithubNotification[] = data.map(notification => ({
        id: notification.id,
        repository: {
          name: notification.repository.name,
          full_name: notification.repository.full_name,
          owner: {
            login: notification.repository.owner.login,
            avatar_url: notification.repository.owner.avatar_url,
          },
        },
        subject: {
          title: notification.subject.title,
          type: notification.subject.type,
          url: notification.subject.url,
          latest_comment_url: notification.subject.latest_comment_url,
        },
        reason: notification.reason,
        unread: notification.unread,
        updated_at: notification.updated_at,
        last_read_at: notification.last_read_at,
        url: notification.url,
        subscription_url: notification.subscription_url,
      }))

      return ctx.json({ notifications }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/latest', async (ctx) => {
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken
    try {
      const pullRequests = await fetchPullRequestsSearchWithCache(
        latestPullRequestsCache,
        latestPullRequestsInflight,
        user.id,
        githubToken,
        LATEST_PULL_REQUESTS_QUERY,
      )
      return ctx.json({ pullRequests }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/need-reviews', async (ctx) => {
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const pullRequests = await fetchPullRequestsSearchWithCache(
        needReviewsPullRequestsCache,
        needReviewsPullRequestsInflight,
        user.id,
        githubToken,
        NEED_REVIEWS_PULL_REQUESTS_QUERY,
      )
      return ctx.json({ pullRequests }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: PullRequestParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
      }

      const data = await fetchGithubPullRequest({ token: githubToken, params })

      const author: GithubPullRequestDetailsAuthor = {
        login: data.user.login,
        avatar_url: data.user.avatar_url,
      }

      let mergeBaseSha = data.base.sha
      const baseRef = data.base.ref
      const headRef = data.head.ref
      const headOwner = data.head.repo.owner.login

      try {
        const compareParams: CompareParams = {
          owner: org,
          repo,
          basehead: `${baseRef}...${headOwner}:${headRef}`,
        }

        const compare = await compareGithubRefs({ token: githubToken, params: compareParams })

        mergeBaseSha = compare.merge_base_commit.sha
      }
      catch {
        mergeBaseSha = data.base.sha
      }

      const pullRequest: GithubPullRequestDetails = {
        number: data.number,
        title: data.title,
        state: data.state,
        draft: Boolean(data.draft),
        created_at: data.created_at,
        updated_at: data.updated_at,
        merged_at: data.merged_at,
        merge_base_sha: mergeBaseSha,
        base_sha: data.base.sha,
        head_sha: data.head.sha,
        base_ref_name: data.base.ref,
        head_ref_name: data.head.ref,
        body: data.body,
        author,
        comments: data.comments,
        review_comments: data.review_comments,
        commits: data.commits,
        additions: data.additions,
        deletions: data.deletions,
        changed_files: data.changed_files,
        labels: data.labels,
        repository: {
          owner: org,
          repo,
        },
        head_repository: {
          owner: data.head.repo.owner.login,
          repo: data.head.repo.name,
        },
      }

      return ctx.json({ pullRequest }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/files', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const files = await fetchGithubPullRequestFilesAllPages({ token: githubToken, params: {
        owner: org,
        repo,
        pull_number: pullNumber,
      } })

      return ctx.json({ files }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/:id/comments', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: PullRequestCommentsParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        per_page: 100,
      }

      const data = await fetchGithubPullRequestComments({ token: githubToken, params })

      const comments: GithubPullRequestReviewComment[]
        = data.map(mapGithubPullRequestReviewComment)

      return ctx.json({ comments }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:id/comments', zValidator(
    'json',
    createPullRequestLineCommentBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const {
      body,
      path,
      commitId,
      line,
      side,
      startLine,
      startSide,
    } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber)) {
      return ctx.json({ error: 'Missing org, repo, or id' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: CreatePullRequestCommentParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        body,
        path,
        commit_id: commitId,
        line,
        side,
        ...(startLine != null ? { start_line: startLine } : {}),
        ...(startSide != null ? { start_side: startSide } : {}),
      }

      const data = await createGithubPullRequestComment({ token: githubToken, params })

      const comment = mapGithubPullRequestReviewComment(data)
      return ctx.json({ comment }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .post('/pr/:prId/comments/:commentId/replies', zValidator(
    'json',
    createPullRequestThreadReplyBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('prId'))
    const inReplyToId = Number(ctx.req.param('commentId'))
    const { body } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber) || Number.isNaN(inReplyToId)) {
      return ctx.json({ error: 'Missing org, repo, prId, or commentId' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: CreatePullRequestCommentReplyParams = {
        owner: org,
        repo,
        pull_number: pullNumber,
        comment_id: inReplyToId,
        body,
      }

      const data = await createGithubPullRequestCommentReply({ token: githubToken, params })

      const comment = mapGithubPullRequestReviewComment(data)
      return ctx.json({ comment }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .delete('/pr/:id/comments/:commentId', async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const commentId = Number(ctx.req.param('commentId'))

    if (!org || !repo || Number.isNaN(pullNumber) || Number.isNaN(commentId)) {
      return ctx.json({ error: 'Missing org, repo, id, or commentId' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: DeletePullRequestCommentParams = {
        owner: org,
        repo,
        comment_id: commentId,
      }

      await deleteGithubPullRequestComment({ token: githubToken, params })

      return ctx.json({ success: true }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .patch('/pr/:id/comments/:commentId', zValidator(
    'json',
    updatePullRequestCommentBodySchema,
  ), async (ctx) => {
    const { org, repo } = ctx.req.query()
    const pullNumber = Number(ctx.req.param('id'))
    const commentId = Number(ctx.req.param('commentId'))
    const { body } = ctx.req.valid('json')

    if (!org || !repo || Number.isNaN(pullNumber) || Number.isNaN(commentId)) {
      return ctx.json({ error: 'Missing org, repo, id, or commentId' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: UpdatePullRequestCommentParams = {
        owner: org,
        repo,
        comment_id: commentId,
        body,
      }

      const data = await patchGithubPullRequestComment({ token: githubToken, params })

      const comment = mapGithubPullRequestReviewComment(data)
      return ctx.json({ comment }, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 403 || status === 404 || status === 422) {
        return ctx.json({ error: (error as Error).message }, status)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/file', async (ctx) => {
    const { org, repo, path, ref } = ctx.req.query()

    if (!org || !repo || !path || !ref) {
      return ctx.json({ error: 'Missing org, repo, path, or ref' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: GetContentParams = {
        owner: org,
        repo,
        path,
        ref,
      }

      const data = await fetchGithubRepositoryContent({ token: githubToken, params })

      let content: string | null = null

      if (typeof data === 'string') {
        content = data
      }
      else if (Buffer.isBuffer(data)) {
        content = data.toString('utf8')
      }
      else if (data && typeof data === 'object' && 'content' in data) {
        const payload = data as { content?: string, encoding?: string }
        if (typeof payload.content === 'string') {
          const encoding = payload.encoding === 'base64' ? 'base64' : 'utf8'
          content = Buffer.from(payload.content, encoding).toString('utf8')
        }
      }

      const payload: GithubFileContent = { content }
      return ctx.json(payload, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ content: null } satisfies GithubFileContent, 200)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/me', async (ctx) => {
    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: UserRepositoriesParams = {
        sort: 'updated',
        direction: 'desc',
        per_page: 100,
      }

      const data = await fetchGithubUserRepositories({ token: githubToken, params })
      const repositories: GithubUserRepository[] = data
        .map(repo => ({
          owner: repo.owner.login,
          repo: repo.name,
          full_name: repo.full_name,
          description: repo.description,
          private: repo.private,
          updated_at: repo.updated_at ?? '',
        }))
        .sort((a, b) => b.updated_at.localeCompare(a.updated_at))

      return ctx.json({ repositories }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: GithubRepositoryParameters = {
        owner,
        repo,
      }

      const data = await fetchGithubRepository({ token: githubToken, params })

      const repositoryDetails: GithubRepositoryDetails = {
        name: data.name,
        full_name: data.full_name,
        description: data.description,
        homepage: data.homepage,
        language: data.language,
        default_branch: data.default_branch,
        stargazers_count: data.stargazers_count,
        forks_count: data.forks_count,
        subscribers_count: data.subscribers_count,
        open_issues_count: data.open_issues_count,
        size: data.size,
        pushed_at: data.pushed_at,
        html_url: data.html_url,
        owner: {
          login: data.owner.login,
          name: data.owner.name || undefined,
          avatar_url: data.owner.avatar_url,
        },
        license: data.license
          ? {
              key: data.license.key,
              name: data.license.name,
              spdx_id: data.license.spdx_id,
            }
          : null,
      }

      return ctx.json(repositoryDetails, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ error: 'Repository not found' }, 404)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/readme', async (ctx) => {
    const { owner, repo } = ctx.req.param()
    const ref = ctx.req.query('ref')

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: GithubRepositoryReadmeParameters = {
        owner,
        repo,
        ...(ref && ref.trim().length > 0 ? { ref } : {}),
      }

      const data = await fetchGithubRepositoryReadme({ token: githubToken, params })

      let content: string | null = null
      if (typeof data.content === 'string') {
        const encoding = data.encoding === 'base64' ? 'base64' : 'utf8'
        content = Buffer.from(data.content, encoding).toString('utf8')
      }
      const path = typeof data.path === 'string' ? data.path : null

      const repositoryReadme: GithubRepositoryReadme = {
        content,
        path,
      }

      return ctx.json(repositoryReadme, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ content: null, path: null } satisfies GithubRepositoryReadme, 200)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/branches', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: GithubRepositoryBranchesParameters = {
        owner,
        repo,
        per_page: 100,
      }

      const data = await fetchGithubRepositoryBranches({ token: githubToken, params })

      const branches: GithubRepositoryBranch[] = data.map(branch => ({
        name: branch.name,
        commit: {
          sha: branch.commit.sha,
          url: branch.commit.url,
        },
        protected: branch.protected,
      }))

      return ctx.json(branches, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ error: 'Repository not found' }, 404)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/trees/:tree_sha', async (ctx) => {
    const { owner, repo, tree_sha } = ctx.req.param()
    const recursive = ctx.req.query('recursive')

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: GithubRepositoryTreeParams = {
        owner,
        repo,
        tree_sha,
        ...(recursive !== undefined ? { recursive } : {}),
      }

      const data = await fetchGithubRepositoryTrees({ token: githubToken, params })

      const tree: GithubRepositoryTree = {
        sha: data.sha,
        url: data.url,
        truncated: data.truncated,
        tree: data.tree,
      }

      return ctx.json(tree, 200)
    }
    catch (error) {
      const status = (error as { status?: number }).status
      if (status === 404) {
        return ctx.json({ error: 'Repository not found' }, 404)
      }
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/pr', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: ListPullsParams = {
        owner,
        repo,
        state: 'all',
        sort: 'updated',
        direction: 'desc',
        per_page: 20,
      }

      const data = await fetchGithubPullRequests({ token: githubToken, params })

      const pullRequests: GithubPullRequest[] = data.map(pull => ({
        number: pull.number,
        title: pull.title,
        state: pull.state,
        draft: Boolean(pull.draft),
        merged_at: pull.merged_at,
        updated_at: pull.updated_at,
        labels: pull.labels.map(label => ({ name: label.name })),
        repository: {
          owner,
          repo,
        },
      }))

      return ctx.json({ pullRequests }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/issues', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: GithubIssueParameters = {
        owner,
        repo,
        state: 'all',
        sort: 'updated',
        direction: 'desc',
        per_page: 20,
      }

      const data = await fetchGithubRepositoryIssues({ token: githubToken, params })

      const issues: GithubIssue[] = data.filter(issue => !issue.pull_request).map(issue => ({
        id: issue.id,
        number: issue.number,
        title: issue.title,
        state: issue.state,
        state_reason: issue.state_reason,
        created_at: issue.created_at,
        updated_at: issue.updated_at,
        closed_at: issue.closed_at,
        labels: issue.labels,
        user: formatGithubUser(issue.user),
        repository: {
          owner,
          repo,
        },
      }))

      return ctx.json({ issues }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/repos/:owner/:repo/issues/:issue_number', async (ctx) => {
    const { owner, repo, issue_number } = ctx.req.param()

    const issueNumber = Number(issue_number)

    if (!owner || !repo || Number.isNaN(issueNumber)) {
      return ctx.json({ error: 'Missing owner, repo, or issue number' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const paramsIssue: GithubIssueDetailsParameters = {
        owner,
        repo,
        issue_number: issueNumber,
      }

      const paramsComments: GithubIssueDetailsCommentParameters = {
        owner,
        repo,
        issue_number: issueNumber,
        per_page: 100,
      }

      const [data, issueComments] = await Promise.all([
        fetchGithubRepositoryIssue({ token: githubToken, params: paramsIssue }),
        fetchGithubRepositoryIssueComments({ token: githubToken, params: paramsComments }),
      ])

      const issue: GithubIssueDetails = {
        id: data.id,
        number: data.number,
        title: data.title,
        state: data.state,
        state_reason: data.state_reason,
        created_at: data.created_at,
        updated_at: data.updated_at,
        closed_at: data.closed_at,
        labels: data.labels,
        body: data.body,
        comments: issueComments.map(comment => ({
          id: comment.id,
          body: comment.body,
          created_at: comment.created_at,
          updated_at: comment.updated_at,
          user: formatGithubUser(comment.user),
        })),
        user: formatGithubUser(data.user),
        repository: {
          owner,
          repo,
        },
      }

      return ctx.json({ issue }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
