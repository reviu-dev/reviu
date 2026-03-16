import type { AuthType, UserContext } from '../lib/auth.js'
import { createMiddleware } from 'hono/factory'
import { auth } from '../lib/auth.js'

function authMiddleware(roleRequired: 'user' | 'admin' | 'pro') {
  return createMiddleware<{ Variables: { user: UserContext } }>(async (c, next) => {
    const session = await auth.api.getSession({ headers: c.req.raw.headers })

    const user = (session?.user as AuthType['user']) ?? null

    if (!user) {
      return c.json({ error: 'Unauthorized' }, 401)
    }

    if (roleRequired === 'admin' && user.role !== 'admin') {
      return c.json({ error: 'Forbidden' }, 403)
    }

    if (roleRequired === 'pro' && !user.proGrantedAt && user.role !== 'admin') {
      return c.json({ error: 'Forbidden' }, 403)
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
}

export const authMiddlewareUser = authMiddleware('user')
export const authMiddlewareAdmin = authMiddleware('admin')
export const authMiddlewarePro = authMiddleware('pro')
