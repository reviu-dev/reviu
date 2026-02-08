import type { AuthType } from '../lib/auth.js'
import { createMiddleware } from 'hono/factory'
import { auth } from '../lib/auth.js'

export const authMiddleware = createMiddleware(async (c, next) => {
  const session = await auth.api.getSession({ headers: c.req.raw.headers })

  const user = (session?.user as AuthType['user']) ?? null

  if (!user) {
    return c.json({ error: 'Unauthorized' }, 401)
  }

  const ghAccessToken = await auth.api.getAccessToken({
    body: {
      providerId: 'github',
    },
    headers: c.req.raw.headers,
  })

  c.set('user', { ...user, github: ghAccessToken })

  await next()
})
