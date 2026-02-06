import type { Hono } from 'hono'

import { authRoutes } from './auth.js'
import { healthcheckRoutes } from './healthcheck.js'
import { userRoutes } from './user.js'

export function routes(app: Hono) {
  app.route('/auth', authRoutes)
  app.route('/users', userRoutes)
  app.route('/healthcheck', healthcheckRoutes)
}
