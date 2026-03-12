# GitHub Backend Cache Plan

## Objective

Build a production-ready cache layer for the GitHub backend that:

- reduces GitHub REST API usage and rate-limit pressure
- keeps the desktop app reactive
- preserves access control for private repositories
- supports horizontal scaling across backend instances
- is ready for a hybrid OAuth user token + GitHub App model

## Current State

Today the backend mostly proxies GitHub requests directly from [backend/src/routes/github.ts](/Users/joris/workspace/reviu/backend/src/routes/github.ts) to [backend/src/services/github.ts](/Users/joris/workspace/reviu/backend/src/services/github.ts).

There is one temporary in-memory cache for:

- `GET /github/pr/latest`
- `GET /github/pr/need-reviews`

That implementation is useful as a proof of direction, but it is not production-ready because it is:

- process-local
- not shared across instances
- not invalidated by mutations
- not backed by Redis
- not revalidated with `ETag` or `If-None-Match`

The desktop currently loads many GitHub endpoints in parallel:

- GitHub home loads pull requests, notifications, and repositories
- repository pages load repository details, branches, readme, code tree, issues, and PRs
- PR pages load PR details, files, commits, issue comments, reviews, review comments, and file contents

This means a route-level cache is not enough. We need a shared cache policy at the GitHub service layer.

## High-Level Architecture

The production architecture should use:

- Redis as the shared cache store
- a small in-process `inflight` map per backend instance to deduplicate concurrent misses
- conditional revalidation against GitHub using `ETag` and `If-None-Match`
- tag-based invalidation after mutations
- stale-while-revalidate behavior for read routes

### Why Redis

Redis is the right default for this backend because:

- the backend is a long-lived Node server, not a serverless function
- we need shared state across instances
- we need low-latency reads
- we need distributed lock patterns to avoid stampedes
- we need explicit invalidation and tag indexes

### Recommended Redis Client

Use `ioredis`.

Reasoning:

- better fit for a long-lived Node backend
- mature reconnect behavior
- good support for pipelines and lock patterns
- better default choice here than HTTP-based Redis clients designed for serverless runtimes

Recommended connection defaults:

- `lazyConnect: true`
- `maxRetriesPerRequest: 1`
- `enableOfflineQueue: false`
- bounded `retryStrategy`

## Cache Scopes

We want 3 scopes.

### 1. Viewer Scope

Cache entries bound to one signed-in user.

Use for:

- notifications
- current user repositories
- "my open PRs"
- "need review" PR search
- any data whose response depends on the current user identity rather than only repo visibility

Key shape:

`gh:viewer:{userId}:{resourceKey}`

Properties:

- never shared across users
- uses OAuth user token
- shortest TTLs

### 2. Installation Scope

Cache entries bound to a GitHub App installation.

Use for:

- private repo and org data once we have a GitHub App installation token
- repo-level and PR-level data that should be shared across authorized users of the same installation

Key shape:

`gh:inst:{installationId}:{resourceKey}`

Properties:

- shared only within the installation
- safest way to share private repo cache
- target scope for most repo and PR reads in the final architecture

### 3. Public Scope

Cache entries for public resources that are safe to share globally.

Use for:

- public repository metadata
- public readme, trees, branches, PR lists, issue lists
- immutable file contents fetched by commit SHA

Key shape:

`gh:public:{resourceKey}`

Properties:

- globally shareable
- longest useful TTLs
- best place to get large cache wins for immutable Git objects

## Authentication Model

The production target should be hybrid, with OAuth as the baseline that always works:

- OAuth user tokens for viewer-scoped routes
- OAuth user tokens as the default fallback for repo-scoped routes
- GitHub App installation tokens only when the repo or org has installed the app

Important product rule:

- no GitHub read route should require a GitHub App installation in order to function
- public repositories must remain accessible without asking an admin to install the app
- private repositories must continue to work through the user's OAuth token even when the app is not installed

### Why Not Full GitHub App Immediately

Some important routes are user-centric and should remain user-scoped:

- `GET /github/notifications`
- `GET /github/repos/me`
- `GET /github/pr/latest`
- `GET /github/pr/need-reviews`

These are tied to the authenticated user and do not map cleanly to a repo installation scope.

### When GitHub App Becomes Better

GitHub App is the better choice for repo and PR routes when we want:

- separate rate-limit budget from user OAuth tokens
- fine-grained repository permissions
- short-lived scoped tokens
- repo or org level installs
- webhook-driven invalidation
- access that is not coupled to one employee's token

GitHub App is therefore an optimization and scaling layer, not the primary compatibility path.

### Viability Constraint

It is not realistic to assume that every public repository owner or organization admin will install the Reviu GitHub App.

That means:

- public repo support must not depend on app installation
- private repo support should not be blocked on app installation either
- the cache system must work well even in an OAuth-only deployment

The app should improve:

- cache sharing for private repos
- webhook-driven freshness
- rate-limit resilience for heavy team usage

But it should never be required for baseline product functionality.

### Practical Decision

Short term:

- keep OAuth user tokens for all routes
- implement Redis cache now
- support public and private repositories without any GitHub App installation requirement
- keep private repo cache viewer-scoped until GitHub App is added
- allow public repo cache to be shared globally even when the source fetch came from an OAuth user token

Target state:

- keep viewer routes on OAuth
- use GitHub App installation tokens for repo and PR routes when the app is installed
- keep OAuth fallback for repo and PR routes when the app is not installed
- move private repo cache from viewer scope to installation scope only for installed repos

## Cache Entry Model

Each cache entry should store:

- `payload`
- `etag`
- `lastModified`
- `fetchedAt`
- `freshUntil`
- `staleUntil`
- `scope`
- `authSource` (`oauth_user` or `github_app_installation`)
- `statusCode`
- minimal GitHub rate-limit metadata for observability

Example TypeScript shape:

```ts
export interface GithubCacheEntry<T> {
  payload: T
  etag?: string
  lastModified?: string
  fetchedAt: number
  freshUntil: number
  staleUntil: number
  scope: 'viewer' | 'installation' | 'public'
  authSource: 'oauth_user' | 'github_app_installation'
  statusCode: number
  githubRateLimit?: {
    limit?: number
    remaining?: number
    reset?: number
    resource?: string
  }
}
```

## Service Layer Design

Introduce a single cache abstraction in the backend service layer instead of duplicating logic in route handlers.

Suggested files:

- `backend/src/lib/redis.ts`
- `backend/src/lib/github-cache.ts`
- `backend/src/lib/github-cache-policy.ts`

Core API:

```ts
type GithubCacheScope = 'viewer' | 'installation' | 'public'

interface GithubCachedRequestOptions<T> {
  key: string
  scope: GithubCacheScope
  ttlMs: number
  staleMs: number
  tags: string[]
  loader: (cached?: GithubCacheEntry<T>) => Promise<GithubUpstreamResult<T>>
}

interface GithubUpstreamResult<T> {
  payload: T
  etag?: string
  lastModified?: string
  statusCode: number
  notModified?: boolean
  githubRateLimit?: {
    limit?: number
    remaining?: number
    reset?: number
    resource?: string
  }
}
```

Behavior:

1. Read Redis.
2. If entry is fresh, return it immediately.
3. If entry is stale-but-allowed, return stale and trigger background refresh if a lock can be acquired.
4. If missing, acquire a short Redis lock and fetch upstream.
5. Use local `inflight` dedupe per process to collapse concurrent misses.
6. Revalidate with `If-None-Match` if `etag` exists.
7. If GitHub returns `304`, refresh timestamps without replacing payload.
8. If GitHub returns `403`, `429`, or `5xx` and stale exists, serve stale.
9. On success, store payload and update tag indexes.

## Redis Data Model

### Cache Value

Primary key:

`gh:cache:{scopeKey}:{resourceKey}`

Serialized value:

- JSON string of `GithubCacheEntry<T>`

### Tag Indexes

Maintain one Redis set per tag:

- `gh:tag:{tag}` -> set of cache keys

This lets us invalidate groups of entries after mutations.

### Locks

Use short-lived Redis locks to avoid stampedes:

- `gh:lock:{scopeKey}:{resourceKey}`

Lock TTL:

- around `5s` to `10s`

## Route Policy Matrix

These values are the initial recommendation, not a permanent contract.

| Backend route | Scope now | Scope target | Auth target | Fresh TTL | Stale window | Tags |
| --- | --- | --- | --- | --- | --- | --- |
| `GET /github/notifications` | viewer | viewer | OAuth user | 10s | 30s | `viewer:{userId}` |
| `GET /github/repos/me` | viewer | viewer | OAuth user | 60s | 5m | `viewer:{userId}` |
| `GET /github/pr/latest` | viewer | viewer | OAuth user | 60s | 5m | `viewer:{userId}` |
| `GET /github/pr/need-reviews` | viewer | viewer | OAuth user | 60s | 5m | `viewer:{userId}` |
| `GET /github/repos/:owner/:repo` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 5m | 30m | `repo:{owner}/{repo}` |
| `GET /github/repos/:owner/:repo/readme?ref=` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 5m | 30m | `repo:{owner}/{repo}`, `ref:{owner}/{repo}:{ref}` |
| `GET /github/repos/:owner/:repo/branches` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 60s | 10m | `repo:{owner}/{repo}` |
| `GET /github/repos/:owner/:repo/trees/:tree_sha` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 10m | 24h | `repo:{owner}/{repo}`, `tree:{owner}/{repo}:{tree_sha}` |
| `GET /github/repos/:owner/:repo/pr` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 30s | 5m | `repo:{owner}/{repo}` |
| `GET /github/repos/:owner/:repo/issues` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 30s | 5m | `repo:{owner}/{repo}` |
| `GET /github/repos/:owner/:repo/issues/:issue_number` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 15s | 2m | `repo:{owner}/{repo}`, `issue:{owner}/{repo}:{issueNumber}` |
| `GET /github/pr/:id` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 15s | 2m | `repo:{owner}/{repo}`, `pr:{owner}/{repo}:{number}` |
| `GET /github/pr/:id/files` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 5m | 1h | `repo:{owner}/{repo}`, `pr:{owner}/{repo}:{number}` |
| `GET /github/pr/:id/commits` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 60s | 10m | `repo:{owner}/{repo}`, `pr:{owner}/{repo}:{number}` |
| `GET /github/pr/:id/issue-comments` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 15s | 2m | `repo:{owner}/{repo}`, `pr:{owner}/{repo}:{number}` |
| `GET /github/pr/:id/reviews` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 15s | 2m | `repo:{owner}/{repo}`, `pr:{owner}/{repo}:{number}` |
| `GET /github/pr/:id/comments` | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 15s | 2m | `repo:{owner}/{repo}`, `pr:{owner}/{repo}:{number}` |
| `GET /github/file?org=&repo=&path=&ref=` where `ref` is branch | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 60s | 10m | `repo:{owner}/{repo}`, `ref:{owner}/{repo}:{ref}` |
| `GET /github/file?org=&repo=&path=&ref=` where `ref` is SHA | viewer | public or installation or viewer | OAuth user by default, GitHub App when installed | 24h | 7d | `repo:{owner}/{repo}`, `blob:{owner}/{repo}:{ref}` |

## Immutable vs Mutable Data

This distinction matters more than "public vs private".

### Best Cache Targets

These give the highest cache win:

- file content by commit SHA
- trees by SHA
- commit files by SHA
- readme by commit or branch if ETag-backed

### Medium Value Targets

- repository details
- branches
- PR list
- issue list

### Short-Lived Targets

- PR details
- issue details
- reviews
- comments
- notifications

## Conditional Revalidation

Every GitHub read helper in [backend/src/services/github.ts](/Users/joris/workspace/reviu/backend/src/services/github.ts) should be updated to optionally return:

- response payload
- HTTP status
- response headers

We need headers for:

- `etag`
- `last-modified`
- `x-ratelimit-limit`
- `x-ratelimit-remaining`
- `x-ratelimit-reset`
- `x-ratelimit-resource`

On refresh:

- if a cache entry has `etag`, send `If-None-Match`
- if GitHub responds `304`, do not replace payload
- only refresh `fetchedAt`, `freshUntil`, and `staleUntil`

This is important because GitHub recommends conditional requests as a way to reduce unnecessary API work and lower rate-limit pressure.

## Invalidation Strategy

Mutations should invalidate cache by tags immediately after success.

### Pull Requests

After:

- `PATCH /github/pr/:id`
- `POST /github/pr/:id/reviews`
- `POST /github/pr/:id/comments`
- `POST /github/pr/:prId/comments/:commentId/replies`
- `PATCH /github/pr/:id/comments/:commentId`
- `DELETE /github/pr/:id/comments/:commentId`

Invalidate:

- `pr:{owner}/{repo}:{number}`
- `repo:{owner}/{repo}`

Optional:

- `viewer:{userId}` for `pr/latest` and `pr/need-reviews` if we want near-immediate home refresh

### Issues

After:

- `PATCH /github/repos/:owner/:repo/issues/:issue_number`
- `POST /github/repos/:owner/:repo/issues/:issue_number/comments`
- `PATCH /github/repos/:owner/:repo/issues/:issue_number/comments/:comment_id`
- `DELETE /github/repos/:owner/:repo/issues/:issue_number/comments/:comment_id`

Invalidate:

- `issue:{owner}/{repo}:{issueNumber}`
- `repo:{owner}/{repo}`

### Branch-Sensitive Reads

When branch or repo metadata changes through webhooks in the future, also invalidate:

- `ref:{owner}/{repo}:{ref}`
- `tree:{owner}/{repo}:{treeSha}`
- `blob:{owner}/{repo}:{sha}`

## Security Rules

### Private Data Must Not Leak

Rules:

- never share viewer-scoped entries across users
- never promote a private repo response into public scope
- never use a global cache key until the repository is known to be public
- negative caching for `404` on private resources must stay scoped to viewer or installation

### Public Promotion

For repository routes, once the repo is confirmed public, entries can be stored in `public` scope.

This promotion must be allowed even if the upstream fetch used an OAuth user token.

In other words:

- public cacheability is determined by repo visibility
- not by whether the source request used OAuth or GitHub App auth

For private repositories:

- keep using viewer scope until GitHub App is available
- then move to installation scope only if the repo is actually installed in the GitHub App

## Observability

Every cached route should expose enough data to understand behavior in production.

### Response Headers

Add debug headers in non-production or behind a feature flag:

- `x-reviu-cache: hit | miss | stale | revalidated | bypass`
- `x-reviu-cache-scope: viewer | installation | public`
- `x-github-ratelimit-remaining`
- `x-github-ratelimit-resource`

### Structured Logs

Log at info or debug level:

- cache key group
- scope
- cache result
- GitHub status
- lock acquired or not
- stale served or not
- revalidation path

### Metrics

Track:

- hit rate per route
- miss rate per route
- stale serve count
- revalidation success count
- GitHub request count
- GitHub `403`, `429`, and `5xx`
- lock contention

## Failure Behavior

Production-ready means degraded service, not blank failure, when GitHub or Redis is unstable.

### If GitHub Fails

If GitHub returns:

- `403`
- `429`
- `5xx`

and stale cache exists:

- serve stale
- log degraded mode

If no stale exists:

- return upstream failure as today

### If Redis Fails

The backend should continue to function without Redis:

- bypass cache
- still use process-local `inflight` dedupe
- log degraded cache mode

Redis outage must not break the GitHub feature entirely.

## GitHub App Rollout Plan

### Phase 1

- keep current OAuth session flow
- add Redis-backed cache
- make the cache architecture fully work in an OAuth-only deployment
- keep private repo data viewer-scoped
- allow public repo data to populate shared public cache

### Phase 1.5

- add auth-selection logic for repo routes:
  - if repo is public, use OAuth fetch and store in public scope
  - if repo is private and app is not installed, use OAuth fetch and store in viewer scope
  - if repo is private and app is installed, use GitHub App fetch and store in installation scope

### Phase 2

- add GitHub App install support
- store installation mapping per repo or org
- fetch installation tokens for repo and PR routes when available
- move private repo cache from viewer scope to installation scope for installed repos only
- never require installation for public repo access
- never hard-fail a repo route only because the app is not installed

### Phase 3

- add GitHub App webhooks
- invalidate cache on:
  - push
  - pull_request
  - pull_request_review
  - pull_request_review_comment
  - issues
  - issue_comment
  - installation_repositories
- reduce TTLs less aggressively because webhook invalidation improves freshness

## Implementation Plan

### Step 1

Add Redis client and configuration:

- `backend/src/lib/redis.ts`
- env vars for Redis URL and optional auth

### Step 2

Create generic cache module:

- `backend/src/lib/github-cache.ts`

Responsibilities:

- key building
- Redis read and write
- local inflight dedupe
- stale-while-revalidate
- lock handling
- tag index management
- invalidation by tag

### Step 3

Refactor GitHub service wrappers in [backend/src/services/github.ts](/Users/joris/workspace/reviu/backend/src/services/github.ts) to return metadata, not only `data`.

### Step 4

Replace the temporary local cache in [backend/src/routes/github.ts](/Users/joris/workspace/reviu/backend/src/routes/github.ts) with calls into the shared cache module.

### Step 5

Apply route policies from the matrix above.

### Step 6

Add invalidation to all mutation endpoints.

### Step 7

Add observability:

- debug headers
- logs
- counters

### Step 8

Add optional GitHub App integration for repo and PR routes, with OAuth fallback preserved.

## Testing Plan

We should add backend tests for the cache module and route integration.

### Unit Tests

Test:

- fresh hit returns cached payload
- stale hit returns stale and schedules refresh
- `ETag` revalidation stores refreshed timestamps on `304`
- stale fallback on upstream error
- lock behavior prevents stampedes
- invalidation by tag removes matching keys
- viewer and public scopes do not collide

### Integration Tests

Test:

- repo route caches and reuses payload
- PR route invalidates after mutation
- file route gets long TTL when `ref` is a SHA
- Redis unavailable path still works by bypassing cache

## Non-Goals

Not in the first iteration:

- caching write responses as source of truth
- trying to share all private repo data globally
- making route handlers manually manage Redis
- replacing desktop local UI caches

## Final Recommendation

The target design is:

- Redis-backed shared cache
- 3 scopes: `viewer`, `installation`, `public`
- hybrid auth model: OAuth as baseline, GitHub App as optional optimization
- conditional revalidation with `ETag`
- tag-based invalidation after mutations
- stale-while-revalidate for responsiveness
- graceful degradation if Redis or GitHub is unhealthy

The product must remain fully usable without any GitHub App installation.

GitHub App should be treated as:

- an optimization for private repos and heavy team usage
- a better source for webhook-driven invalidation
- a way to improve rate-limit posture

It should not be treated as a prerequisite for public repo support or baseline repo access.

This gives the backend a clear path from the current temporary cache to a production-ready architecture without blocking on GitHub App adoption.

## References

- [GitHub rate limits for the REST API](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)
- [GitHub best practices for using the REST API](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api)
- [Differences between GitHub Apps and OAuth apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/differences-between-github-apps-and-oauth-apps)
- [Using webhooks with GitHub Apps](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/using-webhooks-with-github-apps)
- [Hono middleware docs](https://hono.dev/docs/guides/middleware)
- [Hono ETag middleware](https://hono.dev/docs/middleware/builtin/etag)
- [ioredis README](https://github.com/redis/ioredis)
