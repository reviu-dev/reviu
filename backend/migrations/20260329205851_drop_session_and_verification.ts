import type { Knex } from 'knex'

export async function up(knex: Knex): Promise<void> {
  await knex.schema
    .dropTableIfExists('session')
    .dropTableIfExists('verification')
}

export async function down(knex: Knex): Promise<void> {
  await knex.schema
    .createTable('session', (table) => {
      table.text('id').primary()
      table.timestamp('expires_at', { useTz: false }).notNullable()
      table.text('token').notNullable().unique()
      table.timestamp('created_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())
      table.timestamp('updated_at', { useTz: false }).notNullable()
      table.text('ip_address')
      table.text('user_agent')
      table.text('user_id').notNullable().references('id').inTable('user').onDelete('CASCADE')
      table.text('impersonated_by')

      table.index(['user_id'], 'session_userId_idx')
    })
    .createTable('verification', (table) => {
      table.text('id').primary()
      table.text('identifier').notNullable()
      table.text('value').notNullable()
      table.timestamp('expires_at', { useTz: false }).notNullable()
      table.timestamp('created_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())
      table.timestamp('updated_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())

      table.index(['identifier'], 'verification_identifier_idx')
    })
}
