import { zValidator } from '@hono/zod-validator'

import { Hono } from 'hono'
import z from 'zod'
import { auth } from '../lib/auth.js'
import { env } from '../lib/env.js'
import { consumeAuthCode, issueAuthCode } from '../plugins/auth/service.js'
import { desktopSignInErrorPage, desktopSignInPage, desktopSignInSuccessPage } from './auth-pages.js'
import { desktopDeepLinkUrl } from './auth-redirect.js'

const authRouter = new Hono()
const DESKTOP_SOCIAL_SIGN_IN_ENDPOINT = '/api/auth/sign-in/social'
const DESKTOP_SOCIAL_CALLBACK_URL = '/auth/desktop/callback'

const AUTH_PAGE_CSP = [
  'default-src \'none\'',
  'connect-src \'self\'',
  'img-src \'self\'',
  'script-src \'unsafe-inline\'',
  'style-src \'unsafe-inline\'',
  'base-uri \'none\'',
  'form-action \'none\'',
].join('; ')

export const authRoutes = authRouter
  .post('/exchange', zValidator(
    'json',
    z.object({
      code: z.string(),
    }),
  ), async (c) => {
    const { code } = c.req.valid('json')

    const token = await consumeAuthCode(code)

    if (!token) {
      return c.json({ message: 'Invalid or expired code' }, 401)
    }

    return c.json({ token }, 200)
  })
  .get('/desktop/start', (c) => {
    c.header('Content-Security-Policy', AUTH_PAGE_CSP)
    return c.html(desktopSignInPage(DESKTOP_SOCIAL_SIGN_IN_ENDPOINT, DESKTOP_SOCIAL_CALLBACK_URL))
  })
  .get('/desktop/callback', async (c) => {
    const session = await auth.api.getSession({ headers: c.req.raw.headers })

    if (!session) {
      c.header('Content-Security-Policy', AUTH_PAGE_CSP)
      return c.html(desktopSignInErrorPage(), 401)
    }

    const { session: { token } } = session
    const code = await issueAuthCode(token)
    const deepLink = desktopDeepLinkUrl(`/auth/callback?code=${code}`)

    c.header('Content-Security-Policy', AUTH_PAGE_CSP)
    return c.html(desktopSignInSuccessPage(deepLink))
  })
  .get('/web/callback', async (c) => {
    const session = await auth.api.getSession({ headers: c.req.raw.headers })

    if (!session) {
      return c.text('No session found', 401)
    }

    const { session: { token } } = session
    const code = await issueAuthCode(token)

    return c.redirect(`${env.WEB_DASHBOARD_URL}/signin?code=${code}`)
  })
  .get('/subscription', async (c) => {
    return c.redirect(desktopDeepLinkUrl('/subscription/callback'))
  })
