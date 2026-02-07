import type { Endpoints } from '@octokit/types'
import type { UserContext } from '../lib/auth.js'
import { request } from '@octokit/request'
import { Hono } from 'hono'

import { authMiddleware } from '../middlewares/auth.js'

const githubRouter = new Hono()

const DEFAULT_PER_PAGE = 20

type SearchIssuesParams = Endpoints['GET /search/issues']['parameters']
type SearchIssuesResponse = Endpoints['GET /search/issues']['response']['data']
type SearchIssue = SearchIssuesResponse['items'][number]

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

  if (!token) {
    return ctx.json({ error: 'Missing GitHub token' }, 401)
  }

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
