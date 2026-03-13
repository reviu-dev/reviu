import type { UserRoutes } from '#backend'
import { hc } from 'hono/client'
import { env } from '@/lib/env'

import { JWT_KEY } from '@/stores/auth'

export const usersClient = hc<UserRoutes>(`${env.BACKEND_URL}/users`, {
  headers() {
    const token = localStorage.getItem(JWT_KEY)

    return {
      Authorization: token ? `Bearer ${token}` : '',
    }
  },
})
