import { Hono } from 'hono'
import { db } from '../db/index.js'

const healthcheckRouter = new Hono()

export const healthcheckRoutes = healthcheckRouter.get('/', async (ctx) => {
  try {
    await db.execute('SELECT 1')
    return ctx.json({ status: 'ok' }, 200)
  }
  catch (error) {
    return ctx.json({ status: 'error', error: (error as Error).message }, 500)
  }
})
