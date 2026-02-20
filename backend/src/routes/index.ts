import type { Hono } from 'hono'

import { authRoutes } from './auth.js'
import { desktopUpdateRoutes } from './desktop_update.js'
import { githubRoutes } from './github.js'
import { healthcheckRoutes } from './healthcheck.js'
import { userRoutes } from './user.js'

export function routes(app: Hono) {
  app.route('/auth', authRoutes)
  app.route('/desktop/update', desktopUpdateRoutes)
  app.route('/github', githubRoutes)
  app.route('/users', userRoutes)
  app.route('/healthcheck', healthcheckRoutes)
}
