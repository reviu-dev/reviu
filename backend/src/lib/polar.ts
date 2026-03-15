import { Polar } from '@polar-sh/sdk'
import { env } from './env.js'

export const polarClient = new Polar({
  server: env.NODE_ENV === 'production' ? 'production' : 'sandbox',
  accessToken: env.POLAR_ACCESS_TOKEN,
})
