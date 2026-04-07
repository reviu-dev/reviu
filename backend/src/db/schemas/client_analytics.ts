import { index, integer, pgTable, primaryKey, text, timestamp } from 'drizzle-orm/pg-core'

export const clientRouteMetricMinute = pgTable(
  'client_route_metric_minute',
  {
    bucketStart: timestamp('bucket_start').notNull(),
    clientVersion: text('client_version').notNull(),
    clientPlatform: text('client_platform'),
    clientArch: text('client_arch'),
    method: text('method').notNull(),
    route: text('route').notNull(),
    requests: integer('requests').default(0).notNull(),
    uniqueUserIds: text('unique_user_ids').array(),
    lastSeenAt: timestamp('last_seen_at').notNull(),
  },
  table => [
    primaryKey({ columns: [table.bucketStart, table.clientVersion, table.method, table.route] }),
    index('client_route_metric_minute_bucket_start_idx').on(table.bucketStart),
    index('client_route_metric_minute_client_version_idx').on(table.clientVersion),
    index('client_route_metric_minute_route_idx').on(table.route),
  ],
)
