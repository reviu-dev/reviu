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
- `metrics/github-metrics-store.ts`: Postgres persistence, periodic flush, DB overview reader
- `backend/src/routes/github.ts`: GitHub HTTP routes
- `backend/src/routes/admin.ts`: admin overview route for dashboard
- `backend/src/db/schemas/github_metrics.ts`: persisted GitHub metrics tables
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
- `GET /github/pr/:id` updates visibility markers from `base.repo.private`
- a public repo details fetch primes the `public` cache entry so the next user can reuse it

### Metrics Persistence

Metrics are now persisted to Postgres:

- operation metrics by minute
- GitHub resource metrics by minute
- per-user pressure metrics by minute
- current rate-limit state per `userId + resource`

Current behavior:

- the collector keeps in-memory deltas per process
- deltas flush to Postgres every `15s` by default
- shutdown flush is attempted before process exit
- Postgres retention is now handled separately from the live flush loop
- admin overview flushes pending metrics first, then reads Postgres
- if the Postgres read fails, admin overview falls back to the in-memory collector

Persistence goal achieved:

- dashboard continuity across backend restarts
- durable history for TTL tuning
- user joins still happen directly from Postgres

### Conditional Revalidation

`ETag` / `Last-Modified` conditional revalidation is implemented for:

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

Notes:

- paginated PR collections now use explicit change signals:
  - `pr/:id/files` revalidates against the PR validator, or the commit validator when `commitSha` is provided
  - `pr/:id/commits` revalidates against the PR validator
  - `pr/:id/issue-comments` and `pr/:id/comments` revalidate directly against their own resource validators
- `GET /github/repos/:owner/:repo/issues/:issue_number` now uses a strong aggregate strategy:
  - revalidate the issue resource
  - revalidate the issue comments resource
  - keep the aggregate only when both return `304`
  - rebuild the aggregate when either side changes
- paginated issue comments and PR comments now fetch all pages up to a bounded cap:
  - current cap is `10` pages / `1000` items
  - above the cap, the aggregate is intentionally truncated, logged, and tracked in metrics

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

Routes that remain viewer-only by design:

- notifications
- `repos/me`
- search-based user PR endpoints

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
- `GET /admin/github-cache/drilldown`

Current overview includes:

- summary totals
- scope summary (`viewer`, `public`, later `installation`)
- cache status series over time
- GitHub resource series over time
- route aggregates
- users under pressure
- current rate-limit snapshots

Current drilldown includes:

- one selected `operation`
- optional `scope`
- route-level summary
- per-bucket history for `hit`, `stale`, `miss`, `upstream`, pagination, and truncation

Current dashboard behavior also includes:

- click-through drilldown from `Routes Needing Tuning`
- click-through drilldown from `Heavy Paginated Aggregates`
- click-through drilldown from `Truncated Collections`
- scope selector to compare `viewer` vs `public` behavior for the same operation

Current behavior:

- route reads now come from Postgres-backed aggregates
- pending in-memory deltas are flushed before the admin read
- the dashboard survives backend restarts once the metrics tables exist

Current limitation:

- writes still happen per process, then merge in Postgres on flush

### Metrics Retention

Retention v1 is now implemented with simple time-based pruning:

- minute-bucket metric tables are pruned with `DELETE WHERE bucket_start < cutoff`
- current rate-limit state is pruned with `DELETE WHERE updated_at < cutoff`
- retention is configurable through:
  - `GITHUB_METRICS_RETENTION_DAYS` (default `30`)
  - `GITHUB_RATE_LIMIT_STATE_RETENTION_DAYS` (default `14`)
- the intended production path is an external scheduler or Dokploy cron running:
  - `node dist/scripts/prune-github-metrics.js`

Important operational rule:

- periodic flush stays in-process because the collector is process-local memory
- prune belongs in cron; the live flush loop does not

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
| `GET /github/pr/:id` | implemented | viewer or public | yes | 20s | 2m | ETag-backed, now public-promotable |
| `GET /github/pr/:id/files` | implemented | viewer or public | yes | 30s | 5m | uses PR validator, or commit validator when `commitSha` is provided |
| `GET /github/pr/:id/commits` | implemented | viewer or public | yes | 60s | 10m | uses PR validator as the aggregate signal |
| `GET /github/pr/:id/issue-comments` | implemented | viewer or public | yes | 15s | 2m | direct comments validator |
| `GET /github/pr/:id/reviews` | implemented | viewer or public | yes | 20s | 2m | ETag-backed, public promotion active |
| `GET /github/pr/:id/comments` | implemented | viewer or public | yes | 15s | 2m | direct review-comments validator |
| `GET /github/repos/:owner/:repo` | implemented | viewer or public | yes | 2m | 10m | public response primes public cache |
| `GET /github/repos/:owner/:repo/readme` | implemented | viewer or public | yes | 2m | 10m | explicit ref gets separate key |
| `GET /github/repos/:owner/:repo/branches` | implemented | viewer or public | yes | 60s | 5m | ETag-backed |
| `GET /github/repos/:owner/:repo/pr` | implemented | viewer or public | yes | 30s | 5m | conditional list revalidation active |
| `GET /github/repos/:owner/:repo/issues` | implemented | viewer or public | yes | 30s | 5m | conditional list revalidation active |
| `GET /github/repos/:owner/:repo/issues/:issue_number` | implemented | viewer or public | yes | 15s | 2m | strong aggregate revalidation on issue + comments; paginated up to `10` pages / `1000` items |
| `GET /github/repos/:owner/:repo/trees/:tree_sha` | implemented | viewer or public | yes | 10m | 24h | conditional revalidation active |
| `GET /github/file?org=&repo=&path=&ref=` | implemented | viewer or public | yes | 60s or 24h | 10m or 7d | blob SHA variant is still the best cache target |

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

### Metrics Operations

Still missing:

- optional materialized or pre-aggregated views if the dashboard window grows
- operational runbook for `db:push` / migrations and failure monitoring

### Wider Conditional Revalidation

Currently covered on the heavy routes above.

Still worth revisiting later only if metrics justify it:

- any new aggregate read paths introduced later
- more granular multi-page strategies if one validator becomes too coarse

### Issue Details Pagination

Still missing:

- product-visible `truncated` metadata in API responses
- a policy decision on whether very large issue threads should stay fully materialized in one cache entry
- possibly route-specific caps if some aggregates need different headroom

### Public Promotion Quality

Still missing:

- stronger observability around visibility resolution misses
- maybe public-cache debug traces when a viewer miss primes public cache
- maybe explicit viewer-to-public promotion counters

### Installation Scope

Not implemented yet:

- installation-scoped cache keys
- installation token selection
- webhook-driven invalidation for installed repos

## Next Implementation Steps

### 1. Expand Conditional Revalidation Where It Pays Off

Candidates:

- any future multi-call read paths where validators are usable
- finer-grained validators if one aggregate shows poor 304 reuse in metrics

Goal:

- reduce upstream payload transfer on refresh
- keep SWR fast while lowering GitHub pressure further

### 2. Handle Large Threads Better

Now that issue and PR comments paginate correctly up to a bounded cap, the next gap is making truncation explicit to product clients.

Needed:

- expose `truncated` metadata to desktop/web clients
- decide whether to cache the full merged thread or segment it for very large discussions
- keep invalidation behavior predictable for very large discussions

### 3. Improve Metrics Query Hygiene

Now that retention exists, the next ops step is keeping overview queries cheap as history grows.

Needed:

- monitor prune duration and deleted row counts in production
- consider rollups if the dashboard needs windows much larger than minute buckets support comfortably
- watch admin route latency as tables grow

### 4. Improve Dashboard Drilldown

Useful additions:

- explicit public/viewer ratio over time
- resource overlays when correlated with a selected operation become available
- maybe table views for routes with high upstream-to-request ratio

### 5. Add Installation Scope Later

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
- Postgres-backed admin dashboard visibility
- daily-retention pruning via external scheduler

The biggest remaining step is no longer “add cache” or “persist metrics”. It is now “use the persisted data to improve cache policy and revalidation where it buys the most”.
