import { env } from './env.js'

export function getTrustedOrigins() {
  const originsMap: Record<typeof env.NODE_ENV, string[]> = {
    production: ['https://reviu.dev'],
    development: ['*'],
  }

  return originsMap[env.NODE_ENV]
}
