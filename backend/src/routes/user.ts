import { pick } from 'es-toolkit'
import { Hono } from 'hono'
import { authMiddleware } from '../middlewares/auth.js'
import { fetchGithubViewer } from '../services/github.js'

const userRouter = new Hono()

export const userRoutes = userRouter.get('/me', authMiddleware, async (ctx) => {
  const user = ctx.get('user')!
  const formatedUser = pick(user, ['id', 'name', 'email', 'emailVerified', 'image', 'role'])
  const githubToken = user.github.accessToken
  let githubLogin: string | null = null

  try {
    const data = await fetchGithubViewer(githubToken)
    githubLogin = data.login ?? null
  }
  catch {
    githubLogin = null
  }

  return ctx.json({ ...formatedUser, githubLogin }, 200)
})
