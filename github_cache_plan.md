# GitHub Backend Cache Plan

## Objective

Build and evolve a production-ready GitHub cache layer that:

- reduces GitHub REST usage and rate-limit pressure
- keeps the desktop app reactive
- preserves access control for private repositories
- supports multiple backend instances through Redis
- works fully with OAuth-only deployments
- keeps a clean path toward optional GitHub App adoption later

## Current Architecture

The GitHub backend is organized under `backend/src/plugins/github/`:

- `cache/github-cache.ts`: generic cache engine, SWR, inflight dedupe, tag invalidation
- `cache/github-cache-policy.ts`: cache policies, resource keys, tags
- `cache/github-cache-runtime.ts`: shared cache singleton
- `cache/github-repository-visibility.ts`: bounded public-visibility registry
- `service.ts`: GitHub API wrappers, conditional requests, rate-limit extraction
- `metrics/github-metrics.ts`: in-process metrics collector for cache and GitHub pressure
- `metrics/github-metrics-context.ts`: async request context (`userId`, `operation`, `scope`)
- `backend/src/routes/github.ts`: GitHub HTTP routes
- `backend/src/routes/admin.ts`: admin overview route for dashboard
- `backend/src/lib/redis.ts`: Redis-backed cache store with in-memory fallback

## What Is Implemented

### Shared Cache Runtime

Implemented:

- Redis-backed shared store
- in-process `inflight` dedupe per backend instance
- stale-while-revalidate behavior
- tag-based invalidation
- explicit `prime()` support to warm a public cache entry from a viewer fetch
- structured cache logs
- in-memory fallback when Redis is unavailable at runtime

### Cache Scopes

Current design still uses the 3-scope model:

- `viewer`: user-bound cache entries
- `installation`: reserved for future GitHub App installation cache
- `public`: globally shareable entries for public repositories

Current key shapes:

- `gh:cache:viewer:{userId}:{resourceKey}`
- `gh:cache:installation:{installationId}:{resourceKey}`
- `gh:cache:public:{resourceKey}`

### Public Visibility Guard

Public promotion is now implemented behind a bounded visibility registry:

- repo visibility is tracked per `{owner}/{repo}`
- current visibility marker TTL: `2 min`
- if the repo is not known public, routes stay in `viewer`
- if the repo is known public, eligible read routes switch to `public`

Current behavior:

- `GET /github/repos/me` updates visibility markers from repo list payloads
- `GET /github/repos/:owner/:repo` updates visibility markers from repo details
- a public repo details fetch primes the `public` cache entry so the next user can reuse it

### Conditional Revalidation

`ETag` / `Last-Modified` conditional revalidation is implemented for:

- `GET /github/pr/:id`
- `GET /github/pr/:id/reviews`
- `GET /github/repos/:owner/:repo`
- `GET /github/repos/:owner/:repo/readme`
- `GET /github/repos/:owner/:repo/branches`

Notes:

- `GET /github/pr/:id/files` is cached but not conditionally revalidated yet because it aggregates paginated responses
- tree, file, issue list, issue details, and repo PR list are currently cached without conditional revalidation

### Cached Routes

All of these routes already go through the shared cache:

- `GET /github/notifications`
- `GET /github/repos/me`
- `GET /github/pr/latest`
- `GET /github/pr/need-reviews`
- `GET /github/pr/:id`
- `GET /github/pr/:id/files`
- `GET /github/pr/:id/commits`
- `GET /github/pr/:id/issue-comments`
- `GET /github/pr/:id/reviews`
- `GET /github/pr/:id/comments`
- `GET /github/repos/:owner/:repo`
- `GET /github/repos/:owner/:repo/readme`
- `GET /github/repos/:owner/:repo/branches`
- `GET /github/repos/:owner/:repo/pr`
- `GET /github/repos/:owner/:repo/issues`
- `GET /github/repos/:owner/:repo/issues/:issue_number`
- `GET /github/repos/:owner/:repo/trees/:tree_sha`
- `GET /github/file`

### Public Scope Promotion

These repo reads now auto-promote from `viewer` to `public` when repo visibility is known public:

- `GET /github/repos/:owner/:repo`
- `GET /github/repos/:owner/:repo/readme`
- `GET /github/repos/:owner/:repo/branches`
- `GET /github/repos/:owner/:repo/pr`
- `GET /github/repos/:owner/:repo/issues`
- `GET /github/repos/:owner/:repo/issues/:issue_number`
- `GET /github/repos/:owner/:repo/trees/:tree_sha`
- `GET /github/file`

Routes that remain viewer-only by design:

- notifications
- `repos/me`
- search-based user PR endpoints
- all PR detail routes for now

### Invalidation

Mutations already invalidate matching tags after success.

PR-side invalidation currently covers:

- PR search tags
- repo pull-request list tags
- PR details
- PR comments
- PR reviews

Issue-side invalidation currently covers:

- repo issue list tags
- issue details
- issue comments

### Rate-Limit Observability

The GitHub service now parses:

- `x-ratelimit-limit`
- `x-ratelimit-remaining`
- `x-ratelimit-used`
- `x-ratelimit-reset`
- `x-ratelimit-resource`

Current behavior:

- warn logs only when remaining percentage is below threshold
- collector stores current rate-limit state per user and resource
- dashboard overview exposes current rate-limit snapshots

Important rule:

- we use normal response headers for rate-limit status
- we do not poll `GET /rate_limit` routinely

### Admin Metrics Dashboard

Current admin route:

- `GET /admin/github-cache/overview`

Current overview includes:

- summary totals
- scope summary (`viewer`, `public`, later `installation`)
- cache status series over time
- GitHub resource series over time
- route aggregates
- users under pressure
- current rate-limit snapshots

Current limitation:

- the collector is still process-local
- metrics are not yet persisted to Redis or Postgres
- dashboard data resets on backend restart

### Debug Headers

Cached GitHub routes currently return:

- `x-reviu-cache: hit | miss | stale`
- `x-reviu-cache-scope: viewer | public | installation`

## Route Policy Matrix

This matrix reflects the current code, not the original target-only plan.

| Backend route | Status | Active scope | Revalidation | Fresh TTL | Stale window | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `GET /github/notifications` | implemented | viewer | no | 15s | 60s | user-scoped |
| `GET /github/repos/me` | implemented | viewer | no | 60s | 5m | also updates public visibility registry |
| `GET /github/pr/latest` | implemented | viewer | no | 60s | 5m | search bucket sensitive |
| `GET /github/pr/need-reviews` | implemented | viewer | no | 60s | 5m | search bucket sensitive |
| `GET /github/pr/:id` | implemented | viewer | yes | 20s | 2m | ETag-backed |
| `GET /github/pr/:id/files` | implemented | viewer | no | 30s | 5m | `commitSha` variant gets longer TTL |
| `GET /github/pr/:id/commits` | implemented | viewer | no | 60s | 10m | read-only but still user-authorized |
| `GET /github/pr/:id/issue-comments` | implemented | viewer | no | 15s | 2m | tied to PR discussion flow |
| `GET /github/pr/:id/reviews` | implemented | viewer | yes | 20s | 2m | ETag-backed |
| `GET /github/pr/:id/comments` | implemented | viewer | no | 15s | 2m | review comments list |
| `GET /github/repos/:owner/:repo` | implemented | viewer or public | yes | 2m | 10m | public response primes public cache |
| `GET /github/repos/:owner/:repo/readme` | implemented | viewer or public | yes | 2m | 10m | explicit ref gets separate key |
| `GET /github/repos/:owner/:repo/branches` | implemented | viewer or public | yes | 60s | 5m | ETag-backed |
| `GET /github/repos/:owner/:repo/pr` | implemented | viewer or public | no | 30s | 5m | public list promotion active |
| `GET /github/repos/:owner/:repo/issues` | implemented | viewer or public | no | 30s | 5m | public list promotion active |
| `GET /github/repos/:owner/:repo/issues/:issue_number` | implemented | viewer or public | no | 15s | 2m | includes issue comments |
| `GET /github/repos/:owner/:repo/trees/:tree_sha` | implemented | viewer or public | no | 10m | 24h | very strong public-cache candidate |
| `GET /github/file?org=&repo=&path=&ref=` | implemented | viewer or public | no | 60s or 24h | 10m or 7d | blob SHA variant is the best cache target |

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

- `5s`

### Public Visibility Registry

Current key shape:

- `gh:repo-visibility:public:{owner}/{repo}`

Current value:

- `expiresAt`

## Authentication Model

Baseline rule remains unchanged:

- OAuth user tokens must be sufficient for baseline product functionality
- no public repo route should require GitHub App installation
- no private repo route should hard-fail only because a GitHub App is not installed

Current backend behavior:

- OAuth user token for everything
- viewer scope by default
- public scope only for explicitly known public repo reads

Future target:

- viewer routes stay on OAuth
- repo/PR routes may use GitHub App installation tokens when available
- OAuth remains fallback when the app is not installed

## Current Gaps

### Metrics Persistence

Still missing:

- Redis-backed metrics aggregation
- periodic flush to Postgres for historical analysis
- dashboard continuity across backend restarts

### Wider Conditional Revalidation

Still missing for some expensive read routes:

- repo PR list
- repo issue list
- repo issue details
- repo tree
- file by ref or SHA
- possibly selected PR list-like endpoints if worth it

### Public Promotion Quality

Still missing:

- stronger observability around how often routes resolve to `viewer` vs `public`
- maybe dashboard filters by scope and route
- maybe public-cache debug traces when a viewer miss primes public cache

### Installation Scope

Not implemented yet:

- installation-scoped cache keys
- installation token selection
- webhook-driven invalidation for installed repos

## Next Implementation Steps

### 1. Persist Metrics

Move the in-process collector toward Redis-backed aggregation, then optionally flush to Postgres for longer retention.

Why this is next:

- the dashboard is now useful
- but its data is ephemeral
- persistent metrics are needed for real TTL tuning over time

### 2. Expand Conditional Revalidation Where It Pays Off

Candidates:

- repo PR list
- repo issue list
- repo issue details
- file reads where upstream validators are stable enough

Goal:

- reduce payload transfer and upstream work on refresh
- keep SWR responsiveness

### 3. Improve Dashboard Drilldown

Useful additions:

- filter by scope
- filter by operation
- per-route history
- explicit public/viewer ratio over time

### 4. Add Installation Scope Later

If GitHub App is introduced:

- keep OAuth as fallback
- use installation tokens for installed private repos
- share private cache by installation
- use webhooks for better invalidation

## Non-Goals

Not part of the current iteration:

- making GitHub App mandatory
- globally sharing private repo data
- replacing desktop-side local caches
- treating write responses as the cache source of truth

## Final Direction

The backend now has a solid `v1.5` cache posture:

- Redis-backed shared cache
- viewer-first compatibility
- bounded public-scope promotion for safe repo reads
- tag-based invalidation
- stale-while-revalidate responsiveness
- conditional revalidation where it already pays off
- rate-limit observability
- admin dashboard visibility

The biggest remaining step is no longer “add cache”; it is “make the cache observable and durable enough to tune confidently over time”.
