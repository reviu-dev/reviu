import { zValidator } from '@hono/zod-validator'

import { Hono } from 'hono'
import z from 'zod'
import { auth } from '../lib/auth.js'
import { env } from '../lib/env.js'
import { consumeAuthCode, issueAuthCode } from '../plugins/auth/service.js'
import { desktopDeepLinkUrl } from './auth-redirect.js'

const authRouter = new Hono()
const DESKTOP_SOCIAL_SIGN_IN_ENDPOINT = '/api/auth/sign-in/social'
const DESKTOP_SOCIAL_CALLBACK_URL = '/auth/desktop/callback'

function desktopSignInPage() {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Reviu — Sign in with GitHub</title>
    <style>
      *,*::before,*::after{box-sizing:border-box;margin:0;padding:0}
      :root{
        --bg:#0a0a0a;--fg:#fafafa;
        --muted:#a1a1a1;--primary:#2563EB;
        --surface:#161616;--border:#262626;
        --radius:12px;
      }
      body{
        font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;
        background:var(--bg);color:var(--fg);
        display:flex;align-items:center;justify-content:center;
        min-height:100vh;
        -webkit-font-smoothing:antialiased;
      }
      .card{
        display:flex;flex-direction:column;align-items:center;gap:28px;
        padding:48px 40px;
        background:var(--surface);border:1px solid var(--border);
        border-radius:var(--radius);
        max-width:380px;width:100%;
        box-shadow:0 0 0 1px rgba(255,255,255,.03),0 20px 50px -12px rgba(0,0,0,.5);
      }
      .logo{font-size:28px;font-weight:700;letter-spacing:-.03em}
      .logo span{color:var(--primary)}
      .gh-icon{opacity:.85}
      #status{font-size:15px;color:var(--muted);text-align:center;line-height:1.5}
      .spinner{
        width:20px;height:20px;
        border:2px solid var(--border);border-top-color:var(--primary);
        border-radius:50%;
        animation:spin .6s linear infinite;
      }
      @keyframes spin{to{transform:rotate(360deg)}}
      .row{display:flex;align-items:center;gap:10px}
      .error #status{color:#ef4444}
      .error .spinner{display:none}
      .retry{
        display:none;margin-top:4px;
        padding:8px 20px;
        background:var(--primary);color:#fff;
        border:none;border-radius:8px;
        font-size:14px;font-weight:500;cursor:pointer;
        transition:opacity .15s;
      }
      .retry:hover{opacity:.85}
      .error .retry{display:inline-block}
    </style>
  </head>
  <body>
    <div class="card" id="root">
      <div class="logo">Rev<span>iu</span></div>
      <svg class="gh-icon" width="48" height="48" viewBox="0 0 24 24" fill="currentColor">
        <path d="M12 .3a12 12 0 0 0-3.8 23.38c.6.12.82-.26.82-.58l-.01-2.04c-3.34.73-4.04-1.61-4.04-1.61a3.18 3.18 0 0 0-1.33-1.76c-1.09-.74.08-.73.08-.73a2.52 2.52 0 0 1 1.84 1.24 2.56 2.56 0 0 0 3.5 1 2.56 2.56 0 0 1 .76-1.6c-2.67-.3-5.47-1.33-5.47-5.93a4.64 4.64 0 0 1 1.24-3.22 4.32 4.32 0 0 1 .12-3.18s1-.32 3.3 1.23a11.37 11.37 0 0 1 6 0c2.28-1.55 3.29-1.23 3.29-1.23a4.32 4.32 0 0 1 .12 3.18 4.64 4.64 0 0 1 1.23 3.22c0 4.61-2.8 5.63-5.48 5.92a2.86 2.86 0 0 1 .82 2.22l-.01 3.29c0 .32.21.7.82.58A12 12 0 0 0 12 .3"/>
      </svg>
      <div class="row">
        <div class="spinner"></div>
        <p id="status">Connecting to GitHub...</p>
      </div>
      <button class="retry" onclick="location.reload()">Try again</button>
    </div>
    <script>
      const root = document.getElementById('root');
      const statusNode = document.getElementById('status');

      async function startSignIn() {
        try {
          const response = await fetch(${JSON.stringify(DESKTOP_SOCIAL_SIGN_IN_ENDPOINT)}, {
            method: 'POST',
            credentials: 'include',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              provider: 'github',
              callbackURL: ${JSON.stringify(DESKTOP_SOCIAL_CALLBACK_URL)},
              disableRedirect: true,
            }),
          });

          if (!response.ok) throw new Error('Failed to start GitHub sign-in');
          const payload = await response.json();
          if (!payload.url) throw new Error('Missing GitHub sign-in URL');

          statusNode.textContent = 'Redirecting...';
          window.location.replace(payload.url);
        }
        catch (_error) {
          root.classList.add('error');
          statusNode.textContent = 'Could not connect to GitHub. Close this window and try again.';
        }
      }

      void startSignIn();
    </script>
  </body>
</html>`
}

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
    c.header(
      'Content-Security-Policy',
      [
        'default-src \'none\'',
        'connect-src \'self\'',
        'img-src \'self\'',
        'script-src \'unsafe-inline\'',
        'style-src \'unsafe-inline\'',
        'base-uri \'none\'',
        'form-action \'none\'',
      ].join('; '),
    )

    return c.html(desktopSignInPage())
  })
  .get('/desktop/callback', async (c) => {
    const session = await auth.api.getSession({ headers: c.req.raw.headers })

    if (!session) {
      return c.text('No session found', 401)
    }

    const { session: { token } } = session
    const code = await issueAuthCode(token)

    return c.redirect(desktopDeepLinkUrl(`/auth/callback?code=${code}`))
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
