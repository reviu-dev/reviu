import type { AdminRoutes } from '#backend'
import { hc } from 'hono/client'
import { env } from '@/lib/env'

import { LS_BEARER_KEY } from '@/stores/auth'

export const adminClient = hc<AdminRoutes>(`${env.BACKEND_URL}/admin`, {
  headers() {
    const token = localStorage.getItem(LS_BEARER_KEY)

    return {
      Authorization: token ? `Bearer ${token}` : '',
    }
  },
})
