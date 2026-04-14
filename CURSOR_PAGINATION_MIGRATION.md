# Cursor-based pagination migration

## Current state (v1 — offset slicing)

The repo PR and issue lists use server-side slicing: for page N with `per_page` items, the backend requests `first: min(N * per_page, 100)` from GitHub's GraphQL search API and returns only the Nth page slice. `issueCount` provides the total for computing `totalPages`.

This approach is simple but capped at 100 items (GitHub's `first` parameter limit), giving roughly 3 pages of 30 items per state.

## Why cursor pagination

GitHub's GraphQL search API supports cursor-based pagination via `after` / `pageInfo`. Switching to cursors removes the 100-item cap, allowing users to page through all results regardless of count.

## What needs to change

### Backend — `backend/src/plugins/github/service.ts`

1. Add `$after: String` variable to `GITHUB_GRAPHQL_SEARCH_PULL_REQUESTS_QUERY` and `GITHUB_GRAPHQL_SEARCH_ISSUES_QUERY`:
   ```graphql
   search(query: $query, type: ISSUE, first: $first, after: $after) {
     issueCount
     pageInfo {
       endCursor
       hasNextPage
     }
     nodes { ... }
   }
   ```

2. Update `GithubGraphqlSearchPullRequestsResponse` and `GithubGraphqlSearchIssuesResponse` to include `pageInfo: { endCursor: string | null, hasNextPage: boolean }`.

3. Update `fetchGithubPullRequestSearchGraphql` and `fetchGithubIssueSearchGraphql` to accept optional `after?: string` parameter and return `pageInfo`.

### Backend — `backend/src/routes/github.ts`

Replace offset slicing with cursor forwarding in `fetchRepositoryPullRequestsWithCache` and `fetchRepositoryIssuesWithCache`:

- For page 1: call with `first: per_page`, no `after`
- For page N > 1: chain N-1 requests of `first: per_page` using `endCursor` from each response, or fetch `first: (N-1) * per_page` once to get the cursor, then fetch `first: per_page, after: cursor`
- Cache `endCursor` per `(state, filters, page)` so subsequent page navigations can reuse known cursors
- Remove the `min(..., 100)` cap

The `totalPages` calculation changes from `ceil(min(issueCount, 100) / per_page)` to `ceil(issueCount / per_page)`.

### Desktop

No desktop changes needed. The frontend already sends `page` and receives `totalPages` — the backend handles the cursor mechanics transparently.

### Cache considerations

- Cursors are tied to a specific search query at a point in time. If the underlying data changes (new PRs created, issues closed), cursors may become stale.
- Cache TTL already handles this (30s for repo data). When cache expires, cursors are re-fetched.
- Consider storing a cursor chain `Map<page, endCursor>` in memory per `(userId, state, filters)` key, evicted on TTL or filter change.

## Estimated scope

- ~50 lines in `service.ts` (GraphQL query + types + function signatures)
- ~30 lines in `routes/github.ts` (cursor forwarding logic replacing slice logic)
- 0 lines in desktop code
