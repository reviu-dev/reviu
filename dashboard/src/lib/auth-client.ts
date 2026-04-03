import { adminClient } from 'better-auth/client/plugins'
import { createAuthClient } from 'better-auth/vue'
import { LS_BEARER_KEY } from '@/stores/auth'
import { env } from './env'

export const betterAuthClient = createAuthClient({
  baseURL: env.BACKEND_URL,
  fetchOptions: {
    credentials: 'include',
    auth: {
      type: 'Bearer',
      token: () => localStorage.getItem(LS_BEARER_KEY) ?? '',
    },
  },
  plugins: [
    adminClient(),
  ],
})
