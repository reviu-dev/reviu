import type { Knex } from 'knex'

export async function up(knex: Knex): Promise<void> {
  await knex.schema.alterTable('user', (table) => {
    table.text('client_version')
    table.text('client_platform')
    table.text('client_arch')
    table.timestamp('client_version_updated_at')
  })
}

export async function down(knex: Knex): Promise<void> {
  await knex.schema.alterTable('user', (table) => {
    table.dropColumn('client_version')
    table.dropColumn('client_platform')
    table.dropColumn('client_arch')
    table.dropColumn('client_version_updated_at')
  })
}
