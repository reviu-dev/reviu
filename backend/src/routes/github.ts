import type { Endpoints } from '@octokit/types'
import { Buffer } from 'node:buffer'
import { request } from '@octokit/request'
import { Hono } from 'hono'
import { authMiddleware } from '../middlewares/auth.js'

type ListPullsParams = Endpoints['GET /repos/{owner}/{repo}/pulls']['parameters']
type CompareParams
  = Endpoints['GET /repos/{owner}/{repo}/compare/{basehead}']['parameters']

type PullRequestParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['parameters']
type PullRequestCommentsParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['parameters']
type PullRequestCommentResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['response']['data'][number]

type GetContentParams
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['parameters']

type PullRequestDetailsResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['response']['data']

type PullRequestResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls']['response']['data'][number]

interface GithubPullRepository {
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
  repository: GithubPullRepository
}

interface GithubPullRequestDetailsAuthor {
  login: string
  avatarUrl: string
}

interface GithubPullRequestReviewCommentUser {
  login: string
  avatarUrl: string
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
  repository: GithubPullRepository
  headRepository: GithubPullRepository
}

interface GithubPullRequestDiff {
  diff: string
}

interface GithubFileContent {
  content: string | null
}

const githubRouter = new Hono()

githubRouter.use('*', authMiddleware)

export const githubRoutes = githubRouter
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

      const { data } = await request('GET /repos/{owner}/{repo}/pulls', {
        ...params,
        headers: {
          authorization: `Bearer ${githubToken}`,
        },
      })

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

      const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}', {
        ...params,
        headers: {
          authorization: `Bearer ${githubToken}`,
        },
      })

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
        const { data: compare } = await request(
          'GET /repos/{owner}/{repo}/compare/{basehead}',
          {
            ...compareParams,
            headers: {
              authorization: `Bearer ${githubToken}`,
            },
          },
        )

        if (compare.merge_base_commit.sha) {
          mergeBaseSha = compare.merge_base_commit.sha
        }
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
  // TODO: remove this endpoint and fetch files changes from the details endpoint
  .get('/pr/:id/diff', async (ctx) => {
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

      const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}', {
        ...params,
        headers: {
          authorization: `Bearer ${githubToken}`,
          accept: 'application/vnd.github.v3.diff',
        },
      })

      const diff = typeof data === 'string' ? data : JSON.stringify(data)
      const payload: GithubPullRequestDiff = { diff }

      return ctx.json(payload, 200)
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

      const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
        ...params,
        headers: {
          authorization: `Bearer ${githubToken}`,
        },
      })

      const comments: GithubPullRequestReviewComment[] = data.map(comment => ({
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
      }))

      return ctx.json({ comments }, 200)
    }
    catch (error) {
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

      const { data } = await request('GET /repos/{owner}/{repo}/contents/{path}', {
        ...params,
        headers: {
          authorization: `Bearer ${githubToken}`,
          accept: 'application/vnd.github.raw+json',
        },
      })

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
