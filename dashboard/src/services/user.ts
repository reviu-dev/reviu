import type { UserRoutes } from '#backend'
import { hc } from 'hono/client'
import { env } from '@/lib/env'

import { LS_BEARER_KEY } from '@/stores/auth'

export const usersClient = hc<UserRoutes>(`${env.BACKEND_URL}/users`, {
  headers() {
    const token = localStorage.getItem(LS_BEARER_KEY)

    return {
      Authorization: token ? `Bearer ${token}` : '',
    }
  },
})
