import { env } from '../../lib/env.js'

export type FeedbackType = 'bug' | 'feature'

export interface CreateFeedbackParams {
  type: FeedbackType
  title: string
  description: string
  userEmail: string
}

export async function createFeedbackIssue(params: CreateFeedbackParams) {
  const res = await fetch(`${env.SHIPIT_API_URL}/api/v1/issues`, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${env.SHIPIT_API_KEY}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      projectId: env.SHIPIT_PROJECT_ID,
      title: params.title,
      description: params.description,
      labels: ['user-feedback', params.type],
      status: 'backlog',
      metadata: {
        submittedBy: params.userEmail,
      },
    }),
  })

  if (!res.ok) {
    throw new Error(`ShipIt API error: ${res.status}`)
  }

  return res.json()
}
