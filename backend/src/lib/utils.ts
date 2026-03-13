import { env } from './env.js'

export function getTrustedOrigins() {
  const originsMap: Record<typeof env.NODE_ENV, string[]> = {
    production: [env.WEB_DASHBOARD_URL],
    development: [env.WEB_DASHBOARD_URL],
  }

  return originsMap[env.NODE_ENV]
}
