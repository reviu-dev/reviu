import { createStowlineClient } from '@stowline/sdk'

import { env } from './env.js'

export const stowline = createStowlineClient({
  apiKey: env.STOWLINE_API_KEY,
  baseUrl: env.STOWLINE_API_URL,
})
