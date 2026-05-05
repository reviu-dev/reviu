import type { Knex } from 'knex'

export async function up(knex: Knex): Promise<void> {
  await knex.schema
    .createTable('ai_user_setting', (table) => {
      table.text('user_id').primary().references('id').inTable('user').onDelete('CASCADE')
      table.text('credential_mode').notNullable()
      table.text('provider').notNullable()
      table.text('model').notNullable()
      table.text('encrypted_api_key').notNullable()
      table.timestamp('created_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())
      table.timestamp('updated_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())

      table.index(['provider'], 'ai_user_setting_provider_idx')
    })
    .createTable('ai_pr_brief', (table) => {
      table.text('id').primary()
      table.text('user_id').notNullable().references('id').inTable('user').onDelete('CASCADE')
      table.text('owner').notNullable()
      table.text('repo').notNullable()
      table.integer('pull_number').notNullable()
      table.text('head_sha').notNullable()
      table.text('context_hash').notNullable()
      table.text('provider').notNullable()
      table.text('credential_mode').notNullable()
      table.text('model').notNullable()
      table.jsonb('brief_json').notNullable()
      table.timestamp('created_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())
      table.timestamp('updated_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())

      table.unique(
        ['user_id', 'owner', 'repo', 'pull_number', 'head_sha', 'context_hash'],
        { indexName: 'ai_pr_brief_context_unique_idx' },
      )
      table.index(['user_id', 'owner', 'repo', 'pull_number'], 'ai_pr_brief_pr_idx')
      table.index(['created_at'], 'ai_pr_brief_created_at_idx')
    })
    .createTable('ai_usage_event', (table) => {
      table.text('id').primary()
      table.text('user_id').notNullable().references('id').inTable('user').onDelete('CASCADE')
      table.text('task').notNullable()
      table.text('provider').notNullable()
      table.text('credential_mode').notNullable()
      table.text('model').notNullable()
      table.integer('input_tokens')
      table.integer('output_tokens')
      table.text('owner')
      table.text('repo')
      table.integer('pull_number')
      table.timestamp('created_at', { useTz: false }).notNullable().defaultTo(knex.fn.now())

      table.index(['user_id', 'created_at'], 'ai_usage_event_user_created_at_idx')
      table.index(['task'], 'ai_usage_event_task_idx')
    })
}

export async function down(knex: Knex): Promise<void> {
  await knex.schema
    .dropTableIfExists('ai_usage_event')
    .dropTableIfExists('ai_pr_brief')
    .dropTableIfExists('ai_user_setting')
}
