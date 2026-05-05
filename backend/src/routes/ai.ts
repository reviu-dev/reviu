import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { authMiddlewarePro } from '../middlewares/auth.js'
import { aiPrBriefBodySchema, aiSettingsBodySchema } from '../plugins/ai/schemas.js'
import {
  deleteAiSettings,
  generateGithubPrBrief,
  getAiSettings,
  saveAiSettings,
} from '../plugins/ai/service.js'

export const aiRoutes = new Hono()

aiRoutes.use('*', authMiddlewarePro)

function errorStatus(error: unknown) {
  const status = (error as { status?: number }).status
  if (typeof status === 'number' && status >= 400 && status < 600) {
    return status
  }

  return 502
}

function honoStatus(error: unknown) {
  return errorStatus(error) as 400 | 401 | 403 | 404 | 409 | 422 | 429 | 502 | 503
}

aiRoutes
  .get('/settings', async (ctx) => {
    const user = ctx.get('user')!
    return ctx.json({ settings: await getAiSettings(user.id) }, 200)
  })
  .put('/settings', zValidator('json', aiSettingsBodySchema), async (ctx) => {
    const user = ctx.get('user')!

    try {
      const settings = await saveAiSettings(user.id, ctx.req.valid('json'))
      return ctx.json({ settings }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, honoStatus(error))
    }
  })
  .delete('/settings', async (ctx) => {
    const user = ctx.get('user')!
    await deleteAiSettings(user.id)
    return ctx.json({ ok: true }, 200)
  })
  .post('/github/pr/brief', zValidator('json', aiPrBriefBodySchema), async (ctx) => {
    const user = ctx.get('user')!
    const body = ctx.req.valid('json')

    try {
      const brief = await generateGithubPrBrief({
        userId: user.id,
        githubToken: user.github.accessToken,
        owner: body.owner,
        repo: body.repo,
        pullNumber: body.pullNumber,
        forceRefresh: body.forceRefresh,
      })

      return ctx.json({ brief }, 200)
    }
    catch (error) {
      return ctx.json({ error: (error as Error).message }, honoStatus(error))
    }
  })
