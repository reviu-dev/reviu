import type { Hono } from 'hono'

import { userRoutes } from './user.js'

export function routes(app: Hono) {
  app.route('/users', userRoutes)
}
