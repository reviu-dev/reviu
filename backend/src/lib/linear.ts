import { LinearClient } from '@linear/sdk'
import { env } from './env.js'

export const linearClient = new LinearClient({
  apiKey: env.LINEAR_TOKEN,
})
