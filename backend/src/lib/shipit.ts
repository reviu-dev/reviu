import { createShipitClient } from '@shipit-dev/sdk'

import { env } from './env.js'

export const shipit = createShipitClient({
  apiKey: env.SHIPIT_API_KEY,
  baseUrl: env.SHIPIT_API_URL,
})
