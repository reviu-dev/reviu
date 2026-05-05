import { index, integer, jsonb, pgTable, text, timestamp, uniqueIndex } from 'drizzle-orm/pg-core'
import { user } from './auth.js'

export const aiUserSetting = pgTable(
  'ai_user_setting',
  {
    userId: text('user_id')
      .primaryKey()
      .references(() => user.id, { onDelete: 'cascade' }),
    credentialMode: text('credential_mode').notNull(),
    provider: text('provider').notNull(),
    model: text('model').notNull(),
    encryptedApiKey: text('encrypted_api_key').notNull(),
    createdAt: timestamp('created_at').defaultNow().notNull(),
    updatedAt: timestamp('updated_at')
      .defaultNow()
      .$onUpdate(() => /* @__PURE__ */ new Date())
      .notNull(),
  },
  table => [
    index('ai_user_setting_provider_idx').on(table.provider),
  ],
)

export const aiPrBrief = pgTable(
  'ai_pr_brief',
  {
    id: text('id').primaryKey(),
    userId: text('user_id')
      .notNull()
      .references(() => user.id, { onDelete: 'cascade' }),
    owner: text('owner').notNull(),
    repo: text('repo').notNull(),
    pullNumber: integer('pull_number').notNull(),
    headSha: text('head_sha').notNull(),
    contextHash: text('context_hash').notNull(),
    provider: text('provider').notNull(),
    credentialMode: text('credential_mode').notNull(),
    model: text('model').notNull(),
    briefJson: jsonb('brief_json').notNull(),
    createdAt: timestamp('created_at').defaultNow().notNull(),
    updatedAt: timestamp('updated_at')
      .defaultNow()
      .$onUpdate(() => /* @__PURE__ */ new Date())
      .notNull(),
  },
  table => [
    uniqueIndex('ai_pr_brief_context_unique_idx').on(
      table.userId,
      table.owner,
      table.repo,
      table.pullNumber,
      table.headSha,
      table.contextHash,
    ),
    index('ai_pr_brief_pr_idx').on(table.userId, table.owner, table.repo, table.pullNumber),
    index('ai_pr_brief_created_at_idx').on(table.createdAt),
  ],
)

export const aiUsageEvent = pgTable(
  'ai_usage_event',
  {
    id: text('id').primaryKey(),
    userId: text('user_id')
      .notNull()
      .references(() => user.id, { onDelete: 'cascade' }),
    task: text('task').notNull(),
    provider: text('provider').notNull(),
    credentialMode: text('credential_mode').notNull(),
    model: text('model').notNull(),
    inputTokens: integer('input_tokens'),
    outputTokens: integer('output_tokens'),
    owner: text('owner'),
    repo: text('repo'),
    pullNumber: integer('pull_number'),
    createdAt: timestamp('created_at').defaultNow().notNull(),
  },
  table => [
    index('ai_usage_event_user_created_at_idx').on(table.userId, table.createdAt),
    index('ai_usage_event_task_idx').on(table.task),
  ],
)
