import type { Knex } from 'knex'

export async function up(knex: Knex): Promise<void> {
  await knex.schema
    .createTable('user', (table) => {
      table.text('id').primary()
      table.text('name').notNullable()
      table.text('email').notNullable().unique()
      table.boolean('email_verified').notNullable().defaultTo(false)
      table.text('image')
      table.timestamp('created_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())
      table.timestamp('updated_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())
      table.text('role')
      table.boolean('banned').defaultTo(false)
      table.text('ban_reason')
      table.timestamp('ban_expires', { useTz: false })
      table.timestamp('pro_granted_at', { useTz: false })
    })
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
    .createTable('account', (table) => {
      table.text('id').primary()
      table.text('account_id').notNullable()
      table.text('provider_id').notNullable()
      table.text('user_id').notNullable().references('id').inTable('user').onDelete('CASCADE')
      table.text('access_token')
      table.text('refresh_token')
      table.text('id_token')
      table.timestamp('access_token_expires_at', { useTz: false })
      table.timestamp('refresh_token_expires_at', { useTz: false })
      table.text('scope')
      table.text('password')
      table.timestamp('created_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())
      table.timestamp('updated_at', { useTz: false }).notNullable()

      table.index(['user_id'], 'account_userId_idx')
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

export async function down(knex: Knex): Promise<void> {
  await knex.schema
    .dropTableIfExists('verification')
    .dropTableIfExists('account')
    .dropTableIfExists('session')
    .dropTableIfExists('user')
}
