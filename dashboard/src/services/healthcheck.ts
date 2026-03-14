import type { HealthcheckRoutes } from '#backend'
import { hc } from 'hono/client'
import { env } from '@/lib/env'
import { LS_BEARER_KEY } from '@/stores/auth'

export const healthcheckClient = hc<HealthcheckRoutes>(`${env.BACKEND_URL}/healthcheck`, {
  headers() {
    const token = localStorage.getItem(LS_BEARER_KEY)

    return {
      Authorization: token ? `Bearer ${token}` : '',
    }
  },
})
