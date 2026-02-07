import type { Endpoints } from '@octokit/types'
import { request } from '@octokit/request'
import { Hono } from 'hono'

import { authMiddleware } from '../middlewares/auth.js'

const githubRouter = new Hono()

const DEFAULT_PER_PAGE = 20

type SearchIssuesParams = Endpoints['GET /search/issues']['parameters']
type SearchIssuesResponse = Endpoints['GET /search/issues']['response']['data']
type SearchIssue = SearchIssuesResponse['items'][number]
type PullRequestParams
  = Endpoints['GET /repos/{owner}/{repo}/pulls/{pull_number}']['parameters']
type GetContentParams
  = Endpoints['GET /repos/{owner}/{repo}/contents/{path}']['parameters']

interface GithubPullRequestLabel {
  name: string
}

interface GithubPullRequest {
  number: number
  title: string
  state: string
  updatedAt: string
  comments: number
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

interface GithubPullRequestDetails {
  number: number
  title: string
  state: string
  createdAt: string
  updatedAt: string
  mergedAt: string | null
  baseSha: string
  headSha: string
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
}

interface GithubPullRequestDiff {
  diff: string
}

interface GithubFileContent {
  content: string | null
}

function mapLabel(label: SearchIssue['labels'][number]): GithubPullRequestLabel | null {
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
  const token = user.accessToken

  try {
    const { data: viewer } = await request('GET /user', {
      headers: {
        authorization: `Bearer ${token}`,
      },
    })

    const params: SearchIssuesParams = {
      q: `repo:${org}/${repo} is:pr involves:${viewer.login}`,
      sort: 'updated',
      order: 'desc',
      per_page: DEFAULT_PER_PAGE,
    }

    const { data } = await request('GET /search/issues', {
      ...params,
      headers: {
        authorization: `Bearer ${token}`,
      },
    })

    const pullRequests: GithubPullRequest[] = data.items.map(item => ({
      number: item.number,
      title: item.title,
      state: item.state,
      updatedAt: item.updated_at,
      comments: item.comments,
      labels: (item.labels ?? [])
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
  const token = user.accessToken

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

    const pullRequest: GithubPullRequestDetails = {
      number: data.number,
      title: data.title,
      state: data.state,
      createdAt: data.created_at,
      updatedAt: data.updated_at,
      mergedAt: data.merged_at,
      baseSha: data.base?.sha ?? '',
      headSha: data.head?.sha ?? '',
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
    }

    return ctx.json({ pullRequest }, 200)
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
  const token = user.accessToken

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
  const token = user.accessToken

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
