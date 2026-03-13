import { doublePrecision, index, integer, pgTable, primaryKey, text, timestamp } from 'drizzle-orm/pg-core'

export const githubOperationMetricMinute = pgTable(
  'github_operation_metric_minute',
  {
    bucketStart: timestamp('bucket_start').notNull(),
    operation: text('operation').notNull(),
    scope: text('scope').notNull(),
    requests: integer('requests').default(0).notNull(),
    hits: integer('hits').default(0).notNull(),
    staleHits: integer('stale_hits').default(0).notNull(),
    misses: integer('misses').default(0).notNull(),
    upstreamCalls: integer('upstream_calls').default(0).notNull(),
    notModified: integer('not_modified').default(0).notNull(),
    errorCount: integer('error_count').default(0).notNull(),
    nearLimitEvents: integer('near_limit_events').default(0).notNull(),
    totalBackendDurationMs: integer('total_backend_duration_ms').default(0).notNull(),
    totalGithubDurationMs: integer('total_github_duration_ms').default(0).notNull(),
    paginatedLoads: integer('paginated_loads').default(0).notNull(),
    totalPageCount: integer('total_page_count').default(0).notNull(),
    totalItemCount: integer('total_item_count').default(0).notNull(),
    truncatedCount: integer('truncated_count').default(0).notNull(),
    totalPaginationDurationMs: integer('total_pagination_duration_ms').default(0).notNull(),
    ttlMs: integer('ttl_ms'),
    staleMs: integer('stale_ms'),
    lastSeenAt: timestamp('last_seen_at').notNull(),
  },
  table => [
    primaryKey({ columns: [table.bucketStart, table.operation, table.scope] }),
    index('github_operation_metric_minute_bucket_start_idx').on(table.bucketStart),
    index('github_operation_metric_minute_operation_idx').on(table.operation),
  ],
)

export const githubResourceMetricMinute = pgTable(
  'github_resource_metric_minute',
  {
    bucketStart: timestamp('bucket_start').notNull(),
    resource: text('resource').notNull(),
    upstreamCalls: integer('upstream_calls').default(0).notNull(),
    notModified: integer('not_modified').default(0).notNull(),
    errorCount: integer('error_count').default(0).notNull(),
    nearLimitEvents: integer('near_limit_events').default(0).notNull(),
  },
  table => [
    primaryKey({ columns: [table.bucketStart, table.resource] }),
    index('github_resource_metric_minute_bucket_start_idx').on(table.bucketStart),
  ],
)

export const githubUserMetricMinute = pgTable(
  'github_user_metric_minute',
  {
    bucketStart: timestamp('bucket_start').notNull(),
    userId: text('user_id').notNull(),
    requests: integer('requests').default(0).notNull(),
    upstreamCalls: integer('upstream_calls').default(0).notNull(),
    nearLimitEvents: integer('near_limit_events').default(0).notNull(),
    lowestRemainingPct: doublePrecision('lowest_remaining_pct'),
    lastOperation: text('last_operation'),
    lastSeenAt: timestamp('last_seen_at').notNull(),
  },
  table => [
    primaryKey({ columns: [table.bucketStart, table.userId] }),
    index('github_user_metric_minute_bucket_start_idx').on(table.bucketStart),
    index('github_user_metric_minute_user_id_idx').on(table.userId),
  ],
)

export const githubRateLimitState = pgTable(
  'github_rate_limit_state',
  {
    userId: text('user_id').notNull(),
    resource: text('resource').notNull(),
    remaining: integer('remaining'),
    limit: integer('limit'),
    used: integer('used'),
    reset: integer('reset'),
    remainingPct: doublePrecision('remaining_pct'),
    lastOperation: text('last_operation'),
    lastRoute: text('last_route'),
    lastStatus: integer('last_status').notNull(),
    updatedAt: timestamp('updated_at').notNull(),
  },
  table => [
    primaryKey({ columns: [table.userId, table.resource] }),
    index('github_rate_limit_state_updated_at_idx').on(table.updatedAt),
    index('github_rate_limit_state_user_id_idx').on(table.userId),
  ],
)
