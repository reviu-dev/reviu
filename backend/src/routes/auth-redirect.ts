import { env } from '../lib/env.js'

export const DESKTOP_DEEP_LINK_SCHEME: Record<typeof env.NODE_ENV, string> = {
  development: 'reviu-dev',
  production: 'reviu',
}

export function desktopDeepLinkUrl(
  path: string,
) {
  const normalizedPath = path.replace(/^\/+/, '')
  const desktopDeepLinkScheme = DESKTOP_DEEP_LINK_SCHEME[env.NODE_ENV]

  return `${desktopDeepLinkScheme}://${normalizedPath}`
}
