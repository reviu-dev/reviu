import { pick } from 'es-toolkit'
import { Hono } from 'hono'
import { auth } from '../lib/auth.js'
import { env } from '../lib/env.js'
import { logger } from '../lib/logger.js'
import { authMiddlewareUser } from '../middlewares/auth.js'
import { fetchGithubViewer } from '../plugins/github/service.js'

const userRouter = new Hono()

export const userRoutes = userRouter
  .get('/me', authMiddlewareUser, async (ctx) => {
    const user = ctx.get('user')!

    const polarState = await auth.api.state(
      {
        headers: ctx.req.raw.headers,
      },
    )

    const proProductIds = [env.POLAR_SUBSCRIPTION_MONTHLY_PRODUCT_ID, env.POLAR_SUBSCRIPTION_ANNUAL_PRODUCT_ID]
    const activeSubscription = polarState.activeSubscriptions.find(sub => proProductIds.includes(sub.productId)) ?? null

    const hasProAccess = activeSubscription !== null

    if ((hasProAccess && !user.proGrantedAt) || (!hasProAccess && user.proGrantedAt)) {
      // TODO: alert inconsistent state
      logger.error({ userId: user.id, hasProAccess, proGrantedAt: user.proGrantedAt }, 'User subscription state is inconsistent with pro access in database')
    }

    const { url: portalUrl } = await auth.api.portal({
      body: {
        redirect: false,
      },
      headers: ctx.req.raw.headers,
    })

    const githubToken = user.github.accessToken
    let githubLogin: string | null = null

    try {
      const data = await fetchGithubViewer({ token: githubToken })
      githubLogin = data.login ?? null
    }
    catch {
      githubLogin = null
    }

    const formatedUser = {
      ...pick(user, ['id', 'name', 'email', 'emailVerified', 'image', 'role']),
      subscription: {
        portalUrl,
        activeSubscription,
      },
      githubLogin,
    }

    return ctx.json(formatedUser, 200)
  })
