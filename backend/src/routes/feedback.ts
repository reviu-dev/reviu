import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { logger } from '../lib/logger.js'
import { authMiddlewareUser } from '../middlewares/auth.js'
import { createFeedbackIssue } from '../plugins/feedback/service.js'

const feedbackRouter = new Hono()

const feedbackBodySchema = z.object({
  type: z.enum(['bug', 'feature']),
  title: z.string().min(1).max(200),
  description: z.string().max(5000),
})

export const feedbackRoutes = feedbackRouter
  .post('/', authMiddlewareUser, zValidator('json', feedbackBodySchema), async (ctx) => {
    const { type, title, description } = ctx.req.valid('json')
    const user = ctx.get('user')!

    try {
      const result = await createFeedbackIssue({
        type,
        title,
        description,
        userEmail: user.email,
      })
      return ctx.json({ issueId: result.issueId, url: result.url }, 201)
    }
    catch (error) {
      logger.error({ error }, 'Failed to create feedback issue')
      return ctx.json({ error: 'Failed to submit feedback' }, 502)
    }
  })
