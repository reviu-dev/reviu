import { adminClient } from 'better-auth/client/plugins'
import { createAuthClient } from 'better-auth/vue'
import { env } from './env'

export const authClient = createAuthClient({
  baseURL: env.BACKEND_URL,
  plugins: [
    adminClient(),
  ],
})
