import type { Endpoints } from '@octokit/types'
import { Buffer } from 'node:buffer'
import { request } from '@octokit/request'
import { Hono } from 'hono'
import { authMiddleware } from '../middlewares/auth.js'

const githubRouter = new Hono()

const DEFAULT_PER_PAGE = 20

type ListPullsParams = Endpoints['GET /repos/{owner}/{repo}/pulls']['parameters']
type CompareParams
  = Endpoints['GET /repos/{owner}/{repo}/compare/{basehead}']['parameters']

type PullRequestParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['parameters']
type PullRequestCommentsParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['parameters']
type PullRequestCommentsResponse
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}/comments']['response']['data']
type GetContentParams
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['parameters']

interface GithubPullRequestLabel {
  name: string
}

interface GithubPullRequest {
  number: number
  title: string
  state: string
  mergedAt: string | null
  draft: boolean
  updatedAt: string
  labels: GithubPullRequestLabel[]
  repository: {
    owner: string
    repo: string
  }
}

interface GithubPullRequestDetailsAuthor {
  login: string
  avatarUrl: string | null
}

interface GithubPullRequestReviewCommentUser {
  login: string
  avatarUrl: string | null
}

interface GithubPullRequestReviewComment {
  id: number
  pullRequestReviewId: number | null
  diffHunk: string
  path: string
  position: number | null
  originalPosition: number | null
  commitId: string
  originalCommitId: string
  inReplyToId: number | null
  user: GithubPullRequestReviewCommentUser
  body: string
  createdAt: string
  updatedAt: string
  startLine: number | null
  originalStartLine: number | null
  startSide: string | null
  line: number | null
  originalLine: number | null
  side: string | null
}

interface GithubPullRequestDetails {
  number: number
  title: string
  state: string
  draft: boolean
  createdAt: string
  updatedAt: string
  mergedAt: string | null
  mergeBaseSha: string
  baseSha: string
  headSha: string
  baseRefName: string
  headRefName: string
  body: string | null
  author: GithubPullRequestDetailsAuthor
  comments: number
  reviewComments: number
  commits: number
  additions: number
  deletions: number
  changedFiles: number
  labels: GithubPullRequestLabel[]
  repository: {
    owner: string
    repo: string
  }
  headRepository: {
    owner: string
    repo: string
  } | null
}

interface GithubPullRequestDiff {
  diff: string
}

interface GithubFileContent {
  content: string | null
}

type GithubLabel = { name?: string | null } | string | null | undefined

function mapLabel(label: GithubLabel): GithubPullRequestLabel | null {
  if (!label) {
    return null
  }

  if (typeof label === 'string') {
    return {
      name: label,
    }
  }

  if (!label.name) {
    return null
  }

  return {
    name: label.name,
  }
}

export const githubRoutes = githubRouter.get('/pr/latest', authMiddleware, async (ctx) => {
  const { org, repo } = ctx.req.query()

  if (!org || !repo) {
    return ctx.json({ error: 'Missing org or repo' }, 400)
  }

  const user = ctx.get('user')!
  const token = user.github.accessToken

  try {
    const params: ListPullsParams = {
      owner: org,
      repo,
      state: 'all',
      sort: 'updated',
      direction: 'desc',
      per_page: DEFAULT_PER_PAGE,
    }

    const { data } = await request('GET /repos/{owner}/{repo}/pulls', {
      ...params,
      headers: {
        authorization: `Bearer ${token}`,
      },
    })

    const pullRequests: GithubPullRequest[] = data.map(pull => ({
      number: pull.number,
      title: pull.title,
      state: pull.state,
      draft: Boolean(pull.draft),
      mergedAt: pull.merged_at ?? null,
      updatedAt: pull.updated_at,
      labels: (pull.labels ?? [])
        .map(mapLabel)
        .filter((label): label is GithubPullRequestLabel => Boolean(label)),
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

githubRouter.get('/pr/:id', authMiddleware, async (ctx) => {
  const { org, repo } = ctx.req.query()
  const pullNumber = Number(ctx.req.param('id'))

  if (!org || !repo || Number.isNaN(pullNumber)) {
    return ctx.json({ error: 'Missing org, repo, or id' }, 400)
  }

  const user = ctx.get('user')!
  const token = user.github.accessToken

  try {
    const params: PullRequestParams = {
      owner: org,
      repo,
      pull_number: pullNumber,
    }

    const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}', {
      ...params,
      headers: {
        authorization: `Bearer ${token}`,
      },
    })

    const author: GithubPullRequestDetailsAuthor = {
      login: data.user?.login ?? 'unknown',
      avatarUrl: data.user?.avatar_url ?? null,
    }

    let mergeBaseSha = data.base?.sha ?? ''
    const baseRef = data.base?.ref
    const headRef = data.head?.ref
    const headOwner = data.head?.repo?.owner?.login
    if (baseRef && headRef && headOwner) {
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
              authorization: `Bearer ${token}`,
            },
          },
        )

        if (compare.merge_base_commit?.sha) {
          mergeBaseSha = compare.merge_base_commit.sha
        }
      }
      catch {
        mergeBaseSha = data.base?.sha ?? ''
      }
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
      baseSha: data.base?.sha ?? '',
      headSha: data.head?.sha ?? '',
      baseRefName: data.base?.ref ?? '',
      headRefName: data.head?.ref ?? '',
      body: data.body ?? null,
      author,
      comments: data.comments,
      reviewComments: data.review_comments,
      commits: data.commits,
      additions: data.additions,
      deletions: data.deletions,
      changedFiles: data.changed_files,
      labels: (data.labels ?? [])
        .map(mapLabel)
        .filter((label): label is GithubPullRequestLabel => Boolean(label)),
      repository: {
        owner: org,
        repo,
      },
      headRepository: data.head?.repo
        ? {
            owner: data.head.repo.owner?.login ?? org,
            repo: data.head.repo.name ?? repo,
          }
        : null,
    }

    return ctx.json({ pullRequest }, 200)
  }
  catch (error) {
    return ctx.json({ error: (error as Error).message }, 502)
  }
})

githubRouter.get('/pr/:id/comments', authMiddleware, async (ctx) => {
  const { org, repo } = ctx.req.query()
  const pullNumber = Number(ctx.req.param('id'))

  if (!org || !repo || Number.isNaN(pullNumber)) {
    return ctx.json({ error: 'Missing org, repo, or id' }, 400)
  }

  const user = ctx.get('user')!
  const token = user.github.accessToken

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
        authorization: `Bearer ${token}`,
      },
    }) as { data: PullRequestCommentsResponse }

    const comments: GithubPullRequestReviewComment[] = data.map((comment) => ({
      id: comment.id,
      pullRequestReviewId: comment.pull_request_review_id ?? null,
      diffHunk: comment.diff_hunk ?? '',
      path: comment.path,
      position: comment.position ?? null,
      originalPosition: comment.original_position ?? null,
      commitId: comment.commit_id ?? '',
      originalCommitId: comment.original_commit_id ?? '',
      inReplyToId: comment.in_reply_to_id ?? null,
      user: {
        login: comment.user?.login ?? 'unknown',
        avatarUrl: comment.user?.avatar_url ?? null,
      },
      body: comment.body ?? '',
      createdAt: comment.created_at,
      updatedAt: comment.updated_at,
      startLine: comment.start_line ?? null,
      originalStartLine: comment.original_start_line ?? null,
      startSide: comment.start_side ?? null,
      line: comment.line ?? null,
      originalLine: comment.original_line ?? null,
      side: comment.side ?? null,
    }))

    return ctx.json({ comments }, 200)
  }
  catch (error) {
    return ctx.json({ error: (error as Error).message }, 502)
  }
})

githubRouter.get('/file', authMiddleware, async (ctx) => {
  const { org, repo, path, ref } = ctx.req.query()

  if (!org || !repo || !path || !ref) {
    return ctx.json({ error: 'Missing org, repo, path, or ref' }, 400)
  }

  const user = ctx.get('user')!
  const token = user.github.accessToken

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
        authorization: `Bearer ${token}`,
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

githubRouter.get('/pr/:id/diff', authMiddleware, async (ctx) => {
  const { org, repo } = ctx.req.query()
  const pullNumber = Number(ctx.req.param('id'))

  if (!org || !repo || Number.isNaN(pullNumber)) {
    return ctx.json({ error: 'Missing org, repo, or id' }, 400)
  }

  const user = ctx.get('user')!
  const token = user.github.accessToken

  try {
    const params: PullRequestParams = {
      owner: org,
      repo,
      pull_number: pullNumber,
    }

    const { data } = await request('GET /repos/{owner}/{repo}/pulls/{pull_number}', {
      ...params,
      headers: {
        authorization: `Bearer ${token}`,
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
