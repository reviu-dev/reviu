import type { CompareParams, CreatePullRequestCommentParams, CreatePullRequestCommentReplyParams, CreatePullRequestCommentReplyResponse, CreatePullRequestCommentResponse, DeletePullRequestCommentParams, GetContentParams, GithubIssueParameters, GithubIssueResponse, ListPullsParams, NotificationResponse, NotificationsParams, PullRequestCommentResponse, PullRequestCommentsParams, PullRequestDetailsResponse, PullRequestParams, PullRequestResponse, UpdatePullRequestCommentParams, UpdatePullRequestCommentResponse } from '../services/github.js'
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
  fetchGithubRepositoryContent,
  fetchGithubRepositoryIssues,
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
  mergedAt: PullRequestResponse['merged_at']
  updatedAt: PullRequestResponse['updated_at']
  labels: PullRequestResponse['labels']
  repository: GithubRepository
}

interface GithubPullRequestDetailsAuthor {
  login: PullRequestDetailsResponse['user']['login']
  avatarUrl: PullRequestDetailsResponse['user']['avatar_url']
}

interface GithubPullRequestReviewCommentUser {
  login: PullRequestCommentResponse['user']['login']
  avatarUrl: PullRequestCommentResponse['user']['avatar_url']
}

interface GithubPullRequestReviewComment {
  id: PullRequestCommentResponse['id']
  pullRequestReviewId: PullRequestCommentResponse['pull_request_review_id']
  diffHunk: PullRequestCommentResponse['diff_hunk']
  path: PullRequestCommentResponse['path']
  position: PullRequestCommentResponse['position']
  originalPosition: PullRequestCommentResponse['original_position']
  commitId: PullRequestCommentResponse['commit_id']
  originalCommitId: PullRequestCommentResponse['original_commit_id']
  inReplyToId: PullRequestCommentResponse['in_reply_to_id']
  user: GithubPullRequestReviewCommentUser
  body: PullRequestCommentResponse['body']
  createdAt: PullRequestCommentResponse['created_at']
  updatedAt: PullRequestCommentResponse['updated_at']
  startLine: PullRequestCommentResponse['start_line']
  originalStartLine: PullRequestCommentResponse['original_start_line']
  startSide: PullRequestCommentResponse['start_side']
  line: PullRequestCommentResponse['line']
  originalLine: PullRequestCommentResponse['original_line']
  side: PullRequestCommentResponse['side']
}

interface GithubPullRequestDetails {
  number: PullRequestDetailsResponse['number']
  title: PullRequestDetailsResponse['title']
  state: PullRequestDetailsResponse['state']
  draft: NonNullable<PullRequestDetailsResponse['draft']>
  createdAt: PullRequestDetailsResponse['created_at']
  updatedAt: PullRequestDetailsResponse['updated_at']
  mergedAt: PullRequestDetailsResponse['merged_at']
  mergeBaseSha: string
  baseSha: PullRequestDetailsResponse['base']['sha']
  headSha: PullRequestDetailsResponse['head']['sha']
  baseRefName: PullRequestDetailsResponse['base']['ref']
  headRefName: PullRequestDetailsResponse['head']['ref']
  body: PullRequestDetailsResponse['body']
  author: GithubPullRequestDetailsAuthor
  comments: PullRequestDetailsResponse['comments']
  reviewComments: PullRequestDetailsResponse['review_comments']
  commits: PullRequestDetailsResponse['commits']
  additions: PullRequestDetailsResponse['additions']
  deletions: PullRequestDetailsResponse['deletions']
  changedFiles: PullRequestDetailsResponse['changed_files']
  labels: PullRequestDetailsResponse['labels']
  repository: GithubRepository
  headRepository: GithubRepository
}

interface GithubNotificationRepositoryOwner {
  login: NotificationResponse['repository']['owner']['login']
  avatarUrl: NotificationResponse['repository']['owner']['avatar_url']
}

interface GithubNotificationRepository {
  name: NotificationResponse['repository']['name']
  fullName: NotificationResponse['repository']['full_name']
  owner: GithubNotificationRepositoryOwner
}

interface GithubNotificationSubject {
  title: NotificationResponse['subject']['title']
  type: NotificationResponse['subject']['type']
  url: NotificationResponse['subject']['url']
  latestCommentUrl: NotificationResponse['subject']['latest_comment_url']
}

interface GithubNotification {
  id: NotificationResponse['id']
  repository: GithubNotificationRepository
  subject: GithubNotificationSubject
  reason: NotificationResponse['reason']
  unread: NotificationResponse['unread']
  updatedAt: NotificationResponse['updated_at']
  lastReadAt: NotificationResponse['last_read_at']
  url: NotificationResponse['url']
  subscriptionUrl: NotificationResponse['subscription_url']
}

interface GithubIssue {
  id: GithubIssueResponse['id']
  number: GithubIssueResponse['number']
  title: GithubIssueResponse['title']
  state: GithubIssueResponse['state']
  state_reason: GithubIssueResponse['state_reason']
  createdAt: GithubIssueResponse['created_at']
  updatedAt: GithubIssueResponse['updated_at']
  closedAt: GithubIssueResponse['closed_at'] | null
  labels: GithubIssueResponse['labels']
  user: {
    login: NonNullable<GithubIssueResponse['user']>['login']
    name?: NonNullable<GithubIssueResponse['user']>['name']
    avatarUrl: NonNullable<GithubIssueResponse['user']>['avatar_url']
  } | null
  repository: GithubRepository
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

function mapGithubPullRequestReviewComment(
  comment: GithubReviewCommentResponse,
): GithubPullRequestReviewComment {
  return {
    id: comment.id,
    pullRequestReviewId: comment.pull_request_review_id,
    diffHunk: comment.diff_hunk,
    path: comment.path,
    position: comment.position,
    originalPosition: comment.original_position,
    commitId: comment.commit_id,
    originalCommitId: comment.original_commit_id,
    inReplyToId: comment.in_reply_to_id,
    user: {
      login: comment.user.login,
      avatarUrl: comment.user.avatar_url,
    },
    body: comment.body,
    createdAt: comment.created_at,
    updatedAt: comment.updated_at,
    startLine: comment.start_line,
    originalStartLine: comment.original_start_line,
    startSide: comment.start_side,
    line: comment.line,
    originalLine: comment.original_line,
    side: comment.side,
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
          fullName: notification.repository.full_name,
          owner: {
            login: notification.repository.owner.login,
            avatarUrl: notification.repository.owner.avatar_url,
          },
        },
        subject: {
          title: notification.subject.title,
          type: notification.subject.type,
          url: notification.subject.url,
          latestCommentUrl: notification.subject.latest_comment_url,
        },
        reason: notification.reason,
        unread: notification.unread,
        updatedAt: notification.updated_at,
        lastReadAt: notification.last_read_at,
        url: notification.url,
        subscriptionUrl: notification.subscription_url,
      }))

      return ctx.json({ notifications }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/pr/latest', async (ctx) => {
    const { org, repo } = ctx.req.query()

    if (!org || !repo) {
      return ctx.json({ error: 'Missing org or repo' }, 400)
    }

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const params: ListPullsParams = {
        owner: org,
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
        mergedAt: pull.merged_at,
        updatedAt: pull.updated_at,
        labels: pull.labels,
        repository: {
          owner: org,
          repo,
        },
      }))

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
        avatarUrl: data.user.avatar_url,
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
        createdAt: data.created_at,
        updatedAt: data.updated_at,
        mergedAt: data.merged_at,
        mergeBaseSha,
        baseSha: data.base.sha,
        headSha: data.head.sha,
        baseRefName: data.base.ref,
        headRefName: data.head.ref,
        body: data.body,
        author,
        comments: data.comments,
        reviewComments: data.review_comments,
        commits: data.commits,
        additions: data.additions,
        deletions: data.deletions,
        changedFiles: data.changed_files,
        labels: data.labels,
        repository: {
          owner: org,
          repo,
        },
        headRepository: {
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
  .get('/repos/:owner/:repo', async (ctx) => {
    const { owner, repo } = ctx.req.param()

    const user = ctx.get('user')!
    const githubToken = user.github.accessToken

    try {
      const data = await fetchGithubRepository({ token: githubToken, owner, repo })
      return ctx.json(data, 200)
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
        mergedAt: pull.merged_at,
        updatedAt: pull.updated_at,
        labels: pull.labels,
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
        createdAt: issue.created_at,
        updatedAt: issue.updated_at,
        closedAt: issue.closed_at,
        labels: issue.labels,
        user: issue.user
          ? {
              login: issue.user.login,
              name: issue.user.name,
              avatarUrl: issue.user.avatar_url,
            }
          : null,
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
