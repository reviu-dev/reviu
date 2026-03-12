# GitHub Backend Cache Plan

## Objective

Build a production-ready cache layer for the GitHub backend that:

- reduces GitHub REST API usage and rate-limit pressure
- keeps the desktop app reactive
- preserves access control for private repositories
- supports horizontal scaling across backend instances
- stays compatible with an OAuth-only deployment
- leaves a clean path toward optional GitHub App adoption later

## Current Architecture

The GitHub backend is now organized under `backend/src/plugins/github/`:

- `cache/github-cache.ts`: generic cache engine
- `cache/github-cache-policy.ts`: route policy and tag helpers
- `cache/github-cache-runtime.ts`: shared GitHub cache singleton
- `service.ts`: GitHub API wrappers
- `formatter.ts`: response mapping
- `types.ts`: GitHub endpoint and app types
- `backend/src/routes/github.ts`: HTTP routes
- `backend/src/lib/redis.ts`: Redis-backed cache store with in-memory fallback

## What Is Already Implemented

### Shared Cache Runtime

Implemented:

- Redis-backed shared store
- in-process `inflight` dedupe per backend instance
- stale-while-revalidate behavior
- tag-based invalidation
- structured cache logs
- in-memory fallback when Redis is unavailable at runtime

Current files:

- `backend/src/plugins/github/cache/github-cache.ts`
- `backend/src/plugins/github/cache/github-cache-runtime.ts`
- `backend/src/lib/redis.ts`

### Current Cached Routes

These routes already go through the shared cache and return `x-reviu-cache`:

- `GET /github/notifications`
- `GET /github/repos/me`
- `GET /github/pr/latest`
- `GET /github/pr/need-reviews`
- `GET /github/pr/:id`
- `GET /github/pr/:id/files`
- `GET /github/pr/:id/reviews`
- `GET /github/repos/:owner/:repo`
- `GET /github/repos/:owner/:repo/readme`
- `GET /github/repos/:owner/:repo/branches`

Current route wiring lives in:

- `backend/src/routes/github.ts`

### Current Invalidation

Mutations already invalidate matching tags after success for:

- PR body update
- PR review creation
- PR review comment create / reply / edit / delete
- issue body update
- issue comment create / edit / delete

### Current Conditional Revalidation

`ETag` / `Last-Modified` revalidation is implemented for:

- `GET /github/pr/:id`
- `GET /github/pr/:id/reviews`
- `GET /github/repos/:owner/:repo`
- `GET /github/repos/:owner/:repo/readme`
- `GET /github/repos/:owner/:repo/branches`

Current service helpers:

- `fetchGithubPullRequestConditionally`
- `fetchGithubPullRequestReviewsConditionally`
- `fetchGithubRepositoryConditionally`
- `fetchGithubRepositoryReadmeConditionally`
- `fetchGithubRepositoryBranchesConditionally`

Note:

- `GET /github/pr/:id/files` is cached, but not conditionally revalidated yet.
- Reason: it aggregates paginated GitHub responses, so a single upstream `ETag` is not a clean validator for the full assembled payload.

### Current Tests

Focused Vitest coverage exists for:

- fresh hit
- stale serve + background refresh
- stale fallback on upstream error
- tag invalidation
- `304 Not Modified` revalidation path
- policy key/tag generation

Current test files:

- `backend/src/plugins/github/cache/github-cache.test.ts`
- `backend/src/plugins/github/cache/github-cache-policy.test.ts`

## Cache Model

### Scopes

We keep 3 cache scopes in the design.

#### 1. Viewer Scope

Use for responses bound to the signed-in user.

Examples:

- notifications
- current user repositories
- search-based "my PRs" endpoints
- private repo data while we are still OAuth-only

Key shape:

`gh:cache:viewer:{userId}:{resourceKey}`

Properties:

- never shared across users
- works with OAuth user token
- safest default scope today

#### 2. Installation Scope

Target scope for future GitHub App installation tokens.

Examples:

- private repo and PR reads for installed repos
- shared cache across authorized members of the same installation

Key shape:

`gh:cache:installation:{installationId}:{resourceKey}`

Properties:

- not implemented yet
- future-safe target for private shared cache

#### 3. Public Scope

Target scope for globally shareable public resources.

Examples:

- public repository metadata
- public readme
- trees by SHA
- file content by commit SHA

Key shape:

`gh:cache:public:{resourceKey}`

Properties:

- not implemented yet
- should only be used after repo visibility is known to be public

## Authentication Model

The target remains hybrid, but the baseline must always work with OAuth alone.

### Baseline Rule

- GitHub OAuth user tokens must remain sufficient for baseline product functionality.
- No public repo route should require GitHub App installation.
- No private repo route should hard-fail only because the app is not installed.

### Practical Decision

Current backend behavior:

- OAuth user token for everything
- viewer-scoped cache everywhere

Future target:

- viewer routes stay on OAuth
- repo/PR routes can use GitHub App installation tokens when available
- repo/PR routes keep OAuth fallback when app is not installed

## Cache Entry Model

Current cache entries store:

- `payload`
- `tags`
- `fetchedAt`
- `freshUntil`
- `staleUntil`
- `etag`
- `lastModified`

Current TypeScript shape:

```ts
export interface GithubCacheEntry<T> {
  payload: T
  tags: string[]
  fetchedAt: number
  freshUntil: number
  staleUntil: number
  etag?: string
  lastModified?: string
}
```

### Planned Extension

We should later extend the entry or the structured logs with minimal rate-limit metadata:

- `x-ratelimit-limit`
- `x-ratelimit-remaining`
- `x-ratelimit-used`
- `x-ratelimit-reset`
- `x-ratelimit-resource`

This should be treated as observability data, not as the core cache mechanism.

## Why `ETag` and `Last-Modified` Matter

They let us revalidate an expired cache entry without downloading the full payload again.

Flow:

1. store response payload plus `etag` and/or `last-modified`
2. when refreshing, send `If-None-Match` and/or `If-Modified-Since`
3. if GitHub replies `304 Not Modified`, keep the current payload
4. only refresh cache freshness timestamps

This complements stale-while-revalidate:

- fast response to the client
- lower payload transfer
- less avoidable upstream work

## Why `x-ratelimit-*` Headers Matter

These headers are useful, but they do not replace the cache or conditional revalidation.

Good use:

- log them
- expose them in debug headers or metrics
- understand which routes hit which GitHub rate-limit resource bucket
- adapt TTLs or degraded behavior when `remaining` gets very low

Bad use:

- polling `GET /rate_limit` routinely
- driving the whole cache strategy only from rate-limit counters

Recommended rule:

- use the rate-limit headers returned on normal GitHub responses
- avoid routine calls to `GET /rate_limit`

## Redis Data Model

### Cache Values

Primary key:

`gh:cache:{scopeKey}:{resourceKey}`

Value:

- serialized JSON `GithubCacheEntry<T>`

### Tag Indexes

One Redis set per tag:

- `gh:tag:{tag}` -> set of cache keys

### Locks

Short-lived lock key:

- `gh:lock:{cacheKey}`

Current lock TTL:

- around `5s`

## Route Policy Matrix

This matrix reflects current status plus target direction.

| Backend route | Current status | Current scope | Revalidation | Fresh TTL | Stale window | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `GET /github/notifications` | implemented | viewer | no | 15s | 60s | user-scoped |
| `GET /github/repos/me` | implemented | viewer | no | 60s | 5m | user-scoped |
| `GET /github/pr/latest` | implemented | viewer | no | 60s | 5m | search bucket sensitive |
| `GET /github/pr/need-reviews` | implemented | viewer | no | 60s | 5m | search bucket sensitive |
| `GET /github/pr/:id` | implemented | viewer | yes | 20s | 2m | compare call still happens on refresh |
| `GET /github/pr/:id/files` | implemented | viewer | no | 30s | 5m | 10m / 60m when `commitSha` is set |
| `GET /github/pr/:id/commits` | pending | viewer | no | target 60s | target 10m | next candidate |
| `GET /github/pr/:id/issue-comments` | pending | viewer | no | target 15s | target 2m | next candidate |
| `GET /github/pr/:id/reviews` | implemented | viewer | yes | 20s | 2m | ETag-backed |
| `GET /github/pr/:id/comments` | pending | viewer | no | target 15s | target 2m | next candidate |
| `GET /github/repos/:owner/:repo` | implemented | viewer | yes | 2m | 10m | public promotion later |
| `GET /github/repos/:owner/:repo/readme` | implemented | viewer | yes | 2m | 10m | 5m / 30m when `ref` is explicit |
| `GET /github/repos/:owner/:repo/branches` | implemented | viewer | yes | 60s | 5m | ETag-backed |
| `GET /github/repos/:owner/:repo/trees/:tree_sha` | pending | viewer | no | target 10m | target 24h | good public-scope candidate |
| `GET /github/repos/:owner/:repo/pr` | pending | viewer | no | target 30s | target 5m | good next repo list candidate |
| `GET /github/repos/:owner/:repo/issues` | pending | viewer | no | target 30s | target 5m | good next repo list candidate |
| `GET /github/repos/:owner/:repo/issues/:issue_number` | pending | viewer | no | target 15s | target 2m | target with issue comments |
| `GET /github/file?org=&repo=&path=&ref=` | pending | viewer | no | target 60s or 24h | target 10m or 7d | by SHA should eventually become best cache target |

## Tag Strategy

Current tags are centralized in:

- `backend/src/plugins/github/cache/github-cache-policy.ts`

Examples:

- `viewer:{userId}:notifications`
- `viewer:{userId}:repos-me`
- `viewer:{userId}:pr-search`
- `pull-request:{owner}/{repo}:{number}`
- `pull-request:{owner}/{repo}:{number}:files`
- `pull-request:{owner}/{repo}:{number}:reviews`
- `issue:{owner}/{repo}:{number}`
- `repo:{owner}/{repo}:details`
- `repo:{owner}/{repo}:readme`
- `repo:{owner}/{repo}:branches`

## Invalidation Strategy

Mutations should continue to invalidate narrowly scoped tags immediately after success.

### Already Implemented

PR mutations invalidate combinations of:

- PR search tags
- repo pull-request tags
- PR detail tags
- PR comment tags
- PR review tags

Issue mutations invalidate combinations of:

- repo issue tags
- issue detail tags
- issue comment tags

### Still Missing

When we cache more read routes, invalidation must be extended to:

- PR commits
- PR issue comments
- PR review comments list
- repo PR list
- repo issue list
- issue details route

## Observability

### Already Implemented

- `x-reviu-cache: hit | miss | stale`
- structured cache logs when a cache entry resolves
- warning logs on refresh failure and invalidation failure

### Next Observability Step

Add rate-limit metadata capture in `backend/src/plugins/github/service.ts`:

- read `x-ratelimit-limit`
- read `x-ratelimit-remaining`
- read `x-ratelimit-used`
- read `x-ratelimit-reset`
- read `x-ratelimit-resource`

Recommended behavior:

- attach them to structured logs
- optionally expose a subset in debug headers
- avoid calling `GET /rate_limit` routinely

Potential debug headers:

- `x-github-ratelimit-remaining`
- `x-github-ratelimit-resource`

## Failure Behavior

### If GitHub Fails

Current behavior:

- if stale cache exists and refresh fails, serve stale
- if no usable cache exists, return upstream failure

### If Redis Fails

Current behavior:

- fallback to in-memory store
- keep serving requests
- log degraded Redis behavior

Redis outage should not break GitHub features entirely.

## Next Implementation Steps

### 1. Finish Heavy Read Routes

Cache the next expensive routes:

- `GET /github/pr/:id/commits`
- `GET /github/pr/:id/issue-comments`
- `GET /github/pr/:id/comments`
- `GET /github/repos/:owner/:repo/pr`
- `GET /github/repos/:owner/:repo/issues`
- `GET /github/repos/:owner/:repo/issues/:issue_number`
- `GET /github/repos/:owner/:repo/trees/:tree_sha`
- `GET /github/file`

### 2. Add Rate-Limit Observability

Extend service helpers to parse and surface:

- `x-ratelimit-limit`
- `x-ratelimit-remaining`
- `x-ratelimit-used`
- `x-ratelimit-reset`
- `x-ratelimit-resource`

First target:

- logs only

Second target:

- optional debug headers and/or metrics

### 3. Add Public-Scope Promotion

Once repo visibility is known to be public:

- allow repo reads to populate `public` scope
- keep private reads in `viewer` scope until GitHub App exists

Important rule:

- never promote a private response into public scope

### 4. Add Installation Scope Later

If GitHub App is added:

- keep OAuth as fallback
- use installation tokens for installed private repos
- move eligible private repo reads from viewer scope to installation scope
- use webhooks for better invalidation

## Non-Goals

Not part of the current iteration:

- making GitHub App mandatory
- globally sharing private repo data
- turning write responses into the source of truth
- replacing desktop-side local caches

## Final Recommendation

The backend direction remains:

- Redis-backed shared cache
- viewer-first compatibility
- tag-based invalidation
- stale-while-revalidate for responsiveness
- conditional revalidation with `ETag`
- rate-limit observability through response headers
- optional future GitHub App support, never required for baseline product use

This keeps the product usable today while giving a clean path toward better rate-limit posture and stronger cache sharing later.
