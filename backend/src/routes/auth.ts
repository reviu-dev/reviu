import { zValidator } from '@hono/zod-validator'

import { Hono } from 'hono'
import z from 'zod'
import { auth } from '../lib/auth.js'
import { env } from '../lib/env.js'
import { consumeAuthCode, issueAuthCode } from '../plugins/auth/service.js'

const authRouter = new Hono()

export const authRoutes = authRouter
  .post('/exchange', zValidator(
    'json',
    z.object({
      code: z.string(),
    }),
  ), async (c) => {
    const { code } = c.req.valid('json')

    const token = consumeAuthCode(code)

    if (!token) {
      return c.json({ message: 'Invalid or expired code' }, 401)
    }

    return c.json({ token }, 200)
  })
  .get('/desktop/callback', async (c) => {
    const session = await auth.api.getSession({ headers: c.req.raw.headers })

    if (!session) {
      return c.text('No session found', 401)
    }

    const { session: { token } } = session
    const code = issueAuthCode(token)

    return c.redirect(`reviu://auth/callback?code=${code}`)
  })
  .get('/web/callback', async (c) => {
    const session = await auth.api.getSession({ headers: c.req.raw.headers })

    if (!session) {
      return c.text('No session found', 401)
    }

    const { session: { token } } = session
    const code = issueAuthCode(token)

    return c.redirect(`${env.WEB_DASHBOARD_URL}/signin?code=${code}`)
  })
  .get('/subscription', async (c) => {
    return c.redirect('reviu://subscription/callback')
  })
