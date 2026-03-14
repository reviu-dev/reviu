import type { adminRoutes } from './routes/admin.js'
import type { authRoutes } from './routes/auth.js'
import type { healthcheckRoutes } from './routes/healthcheck.js'
import type { userRoutes } from './routes/user.js'

export type AdminRoutes = typeof adminRoutes
export type UserRoutes = typeof userRoutes
export type AuthRoutes = typeof authRoutes
export type HealthcheckRoutes = typeof healthcheckRoutes
