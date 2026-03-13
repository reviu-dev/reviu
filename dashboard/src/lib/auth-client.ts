import { adminClient } from 'better-auth/client/plugins'
import { createAuthClient } from 'better-auth/vue'
import { JWT_KEY } from '@/stores/auth'
import { env } from './env'

export const betterAuthClient = createAuthClient({
  baseURL: env.BACKEND_URL,
  fetchOptions: {
    credentials: 'omit',
    auth: {
      type: 'Bearer',
      token: () => localStorage.getItem(JWT_KEY) ?? '',
    },
  },
  plugins: [
    adminClient(),
  ],
})
