import { env } from '../../lib/env.js'
import { linearClient } from '../../lib/linear.js'
import { logger } from '../../lib/logger.js'

export type FeedbackType = 'bug' | 'feature'

export interface CreateFeedbackParams {
  type: FeedbackType
  title: string
  description: string
  userEmail: string
}

const labelCache = new Map<string, string>()

async function getLabelIds(names: string[]): Promise<string[]> {
  const missing = names.filter(name => !labelCache.has(name))

  if (missing.length > 0) {
    const team = await linearClient.team(env.LINEAR_TEAM_ID)
    const labels = await team.labels()

    for (const label of labels.nodes) {
      labelCache.set(label.name, label.id)
    }
  }

  return names
    .map(name => labelCache.get(name))
    .filter((id): id is string => id !== undefined)
}

export async function createFeedbackIssue(params: CreateFeedbackParams): Promise<{ issueId: string, url: string }> {
  const labelIds = await getLabelIds(['user-feedback', params.type])

  const description = `${params.description}\n\n---\n**Submitted by:** ${params.userEmail}`

  logger.info({ type: params.type, title: params.title, userEmail: params.userEmail }, 'Creating feedback issue')

  const payload = await linearClient.createIssue({
    teamId: env.LINEAR_TEAM_ID,
    title: params.title,
    description,
    labelIds,
  })

  const issue = await payload.issue

  if (!issue) {
    throw new Error('Failed to create Linear issue')
  }

  return { issueId: issue.id, url: issue.url }
}
