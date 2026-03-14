import type { Knex } from 'knex'

export async function up(knex: Knex): Promise<void> {
  await knex.schema
    .createTable('github_operation_metric_minute', (table) => {
      table.timestamp('bucket_start', { useTz: false }).notNullable()
      table.text('operation').notNullable()
      table.text('scope').notNullable()
      table.integer('requests').notNullable().defaultTo(0)
      table.integer('hits').notNullable().defaultTo(0)
      table.integer('stale_hits').notNullable().defaultTo(0)
      table.integer('misses').notNullable().defaultTo(0)
      table.integer('upstream_calls').notNullable().defaultTo(0)
      table.integer('not_modified').notNullable().defaultTo(0)
      table.integer('error_count').notNullable().defaultTo(0)
      table.integer('near_limit_events').notNullable().defaultTo(0)
      table.integer('total_backend_duration_ms').notNullable().defaultTo(0)
      table.integer('total_github_duration_ms').notNullable().defaultTo(0)
      table.integer('paginated_loads').notNullable().defaultTo(0)
      table.integer('total_page_count').notNullable().defaultTo(0)
      table.integer('total_item_count').notNullable().defaultTo(0)
      table.integer('truncated_count').notNullable().defaultTo(0)
      table.integer('total_pagination_duration_ms').notNullable().defaultTo(0)
      table.integer('ttl_ms')
      table.integer('stale_ms')
      table.timestamp('last_seen_at', { useTz: false }).notNullable()

      table.primary(['bucket_start', 'operation', 'scope'])
      table.index(['bucket_start'], 'github_operation_metric_minute_bucket_start_idx')
      table.index(['operation'], 'github_operation_metric_minute_operation_idx')
    })
    .createTable('github_resource_metric_minute', (table) => {
      table.timestamp('bucket_start', { useTz: false }).notNullable()
      table.text('resource').notNullable()
      table.integer('upstream_calls').notNullable().defaultTo(0)
      table.integer('not_modified').notNullable().defaultTo(0)
      table.integer('error_count').notNullable().defaultTo(0)
      table.integer('near_limit_events').notNullable().defaultTo(0)

      table.primary(['bucket_start', 'resource'])
      table.index(['bucket_start'], 'github_resource_metric_minute_bucket_start_idx')
    })
    .createTable('github_user_metric_minute', (table) => {
      table.timestamp('bucket_start', { useTz: false }).notNullable()
      table.text('user_id').notNullable()
      table.integer('requests').notNullable().defaultTo(0)
      table.integer('upstream_calls').notNullable().defaultTo(0)
      table.integer('near_limit_events').notNullable().defaultTo(0)
      table.double('lowest_remaining_pct')
      table.text('last_operation')
      table.timestamp('last_seen_at', { useTz: false }).notNullable()

      table.primary(['bucket_start', 'user_id'])
      table.index(['bucket_start'], 'github_user_metric_minute_bucket_start_idx')
      table.index(['user_id'], 'github_user_metric_minute_user_id_idx')
    })
    .createTable('github_rate_limit_state', (table) => {
      table.text('user_id').notNullable()
      table.text('resource').notNullable()
      table.integer('remaining')
      table.integer('limit')
      table.integer('used')
      table.integer('reset')
      table.double('remaining_pct')
      table.text('last_operation')
      table.text('last_route')
      table.integer('last_status').notNullable()
      table.timestamp('updated_at', { useTz: false }).notNullable()

      table.primary(['user_id', 'resource'])
      table.index(['updated_at'], 'github_rate_limit_state_updated_at_idx')
      table.index(['user_id'], 'github_rate_limit_state_user_id_idx')
    })
}

export async function down(knex: Knex): Promise<void> {
  await knex.schema
    .dropTableIfExists('github_rate_limit_state')
    .dropTableIfExists('github_user_metric_minute')
    .dropTableIfExists('github_resource_metric_minute')
    .dropTableIfExists('github_operation_metric_minute')
}
