import { pick } from 'es-toolkit'
import { Hono } from 'hono'
import { auth } from '../lib/auth.js'
import { env } from '../lib/env.js'
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

    const activeSubscription = polarState.activeSubscriptions.find(sub => sub.productId === env.POLAR_SUBSCRIPTION_PRODUCT_ID) ?? null

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
