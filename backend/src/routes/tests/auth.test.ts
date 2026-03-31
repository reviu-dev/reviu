import { describe, expect, it } from 'vitest'

async function request(path: string, init?: RequestInit) {
  const { authRoutes } = await import('../auth.js')
  return authRoutes.request(`http://localhost${path}`, init)
}

describe('auth routes', () => {
  it('serves a browser-started desktop sign-in page', async () => {
    const response = await request('/desktop/start')

    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toContain('text/html')
    expect(response.headers.get('content-security-policy')).toContain('script-src \'unsafe-inline\'')

    const html = await response.text()

    expect(html).toContain('/api/auth/sign-in/social')
    expect(html).toContain('/auth/desktop/callback')
    expect(html).toContain('provider: \'github\'')
  })
})
