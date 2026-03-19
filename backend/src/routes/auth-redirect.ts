import { env } from '../lib/env.js'

export const DESKTOP_DEEP_LINK_SCHEME: Record<typeof env.NODE_ENV, string> = {
  development: 'reviu-dev',
  production: 'reviu',
}

export function desktopDeepLinkSchemeForNodeEnv(nodeEnv: typeof env.NODE_ENV) {
  return DESKTOP_DEEP_LINK_SCHEME[nodeEnv]
}

export function desktopDeepLinkUrlForNodeEnv(
  nodeEnv: typeof env.NODE_ENV,
  path: string,
) {
  const normalizedPath = path.replace(/^\/+/, '')
  const desktopDeepLinkScheme = desktopDeepLinkSchemeForNodeEnv(nodeEnv)

  return `${desktopDeepLinkScheme}://${normalizedPath}`
}

export function desktopDeepLinkUrl(
  path: string,
) {
  return desktopDeepLinkUrlForNodeEnv(env.NODE_ENV, path)
}
