import type { Knex } from 'knex'

export async function up(knex: Knex): Promise<void> {
  await knex.schema
    .createTable('client_route_metric_minute', (table) => {
      table.timestamp('bucket_start', { useTz: false }).notNullable()
      table.text('client_version').notNullable()
      table.text('client_platform')
      table.text('client_arch')
      table.text('method').notNullable()
      table.text('route').notNullable()
      table.integer('requests').notNullable().defaultTo(0)
      table.specificType('unique_user_ids', 'text[]')
      table.timestamp('last_seen_at', { useTz: false }).notNullable()

      table.primary(['bucket_start', 'client_version', 'method', 'route'])
      table.index(['bucket_start'], 'client_route_metric_minute_bucket_start_idx')
      table.index(['client_version'], 'client_route_metric_minute_client_version_idx')
      table.index(['route'], 'client_route_metric_minute_route_idx')
    })
}

export async function down(knex: Knex): Promise<void> {
  await knex.schema.dropTableIfExists('client_route_metric_minute')
}
