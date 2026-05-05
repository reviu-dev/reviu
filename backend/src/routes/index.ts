import type { Hono } from 'hono'

import { adminRoutes } from './admin.js'
import { aiRoutes } from './ai.js'
import { assetsRoutes } from './assets.js'
import { authRoutes } from './auth.js'
import { crashReportRoutes } from './crash_reports.js'
import { desktopUpdateRoutes } from './desktop_update.js'
import { feedbackRoutes } from './feedback.js'
import { githubRoutes } from './github.js'
import { healthcheckRoutes } from './healthcheck.js'
import { userRoutes } from './user.js'

export function routes(app: Hono) {
  app.route('/admin', adminRoutes)
  app.route('/ai', aiRoutes)
  app.route('/assets', assetsRoutes)
  app.route('/auth', authRoutes)
  app.route('/crash-reports', crashReportRoutes)
  app.route('/desktop/update', desktopUpdateRoutes)
  app.route('/feedback', feedbackRoutes)
  app.route('/github', githubRoutes)
  app.route('/users', userRoutes)
  app.route('/healthcheck', healthcheckRoutes)
}
