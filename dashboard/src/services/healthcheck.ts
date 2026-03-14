import type { HealthcheckRoutes } from '#backend'
import { hc } from 'hono/client'
import { env } from '@/lib/env'

export const healthcheckClient = hc<HealthcheckRoutes>(`${env.BACKEND_URL}/healthcheck`)
