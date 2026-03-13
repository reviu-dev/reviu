import type { AuthRoutes } from '#backend'
import { hc } from 'hono/client'
import { env } from '@/lib/env'

export const authClient = hc<AuthRoutes>(`${env.BACKEND_URL}/auth`)
