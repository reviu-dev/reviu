import type { authRoutes } from './routes/auth.js'
import type { userRoutes } from './routes/user.js'

export type UserRoutes = typeof userRoutes
export type AuthRoutes = typeof authRoutes
