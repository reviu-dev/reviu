import { pick } from 'es-toolkit'
import { Hono } from 'hono'
import { authMiddleware } from '../middlewares/user.js'

const userRouter = new Hono()

export const userRoutes = userRouter.get('/me', authMiddleware, async (ctx) => {
  const user = ctx.get('user')!
  const formatedUser = pick(user, ['id', 'name', 'email', 'emailVerified', 'image', 'role'])

  return ctx.json(formatedUser, 200)
})
