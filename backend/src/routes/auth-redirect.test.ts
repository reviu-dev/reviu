import { describe, expect, it } from 'vitest'

import {
  DESKTOP_DEEP_LINK_SCHEME,
  desktopDeepLinkSchemeForNodeEnv,
  desktopDeepLinkUrlForNodeEnv,
} from './auth-redirect.js'

describe('auth redirect helpers', () => {
  it('maps development to the dev desktop scheme', () => {
    expect(DESKTOP_DEEP_LINK_SCHEME.development).toBe('reviu-dev')
    expect(desktopDeepLinkSchemeForNodeEnv('development')).toBe('reviu-dev')
  })

  it('maps production to the prod desktop scheme', () => {
    expect(DESKTOP_DEEP_LINK_SCHEME.production).toBe('reviu')
    expect(desktopDeepLinkSchemeForNodeEnv('production')).toBe('reviu')
  })

  it('builds deep link urls with normalized paths', () => {
    expect(desktopDeepLinkUrlForNodeEnv('development', '/auth/callback?code=abc')).toBe(
      'reviu-dev://auth/callback?code=abc',
    )
    expect(desktopDeepLinkUrlForNodeEnv('production', 'subscription/callback')).toBe(
      'reviu://subscription/callback',
    )
  })
})
