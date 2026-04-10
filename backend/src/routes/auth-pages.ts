import { readFileSync } from 'node:fs'

const LOGO_LIGHT = readFileSync(
  new URL('../assets/reviu_logo_light.svg', import.meta.url),
  'utf8',
).trim()
const LOGO_DARK = readFileSync(
  new URL('../assets/reviu_logo_dark.svg', import.meta.url),
  'utf8',
).trim()
const LOGO_MARKUP = `
  <div class="logo logo--light">${LOGO_DARK}</div>
  <div class="logo logo--dark">${LOGO_LIGHT}</div>
`

const SHARED_STYLES = `
  *,*::before,*::after{box-sizing:border-box;margin:0;padding:0}
  :root{
    --bg:oklch(1 0 0);--fg:oklch(0.145 0 0);
    --muted:oklch(0.556 0 0);--primary:#2563EB;
    --surface:oklch(0.97 0 0);--border:oklch(0.922 0 0);
    --radius:0.625rem;
    color-scheme:light dark;
  }
  @media(prefers-color-scheme:dark){
    :root{
      --bg:oklch(0.145 0 0);--fg:oklch(0.985 0 0);
      --muted:oklch(0.708 0 0);
      --surface:oklch(0.269 0 0);--border:oklch(0.269 0 0);
    }
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
  }
  .logo{display:block;width:140px;height:auto}
  .logo svg{display:block;width:100%;height:auto}
  .logo--dark{display:none}
  @media(prefers-color-scheme:dark){
    .logo--light{display:none}
    .logo--dark{display:block}
  }
  .gh-icon{opacity:.85}
  #status{font-size:15px;color:var(--muted);text-align:center;line-height:1.5}
  .row{display:flex;align-items:center;gap:10px}
`

const GITHUB_ICON = `<svg class="gh-icon" width="48" height="48" viewBox="0 0 24 24" fill="currentColor">
  <path d="M12 .3a12 12 0 0 0-3.8 23.38c.6.12.82-.26.82-.58l-.01-2.04c-3.34.73-4.04-1.61-4.04-1.61a3.18 3.18 0 0 0-1.33-1.76c-1.09-.74.08-.73.08-.73a2.52 2.52 0 0 1 1.84 1.24 2.56 2.56 0 0 0 3.5 1 2.56 2.56 0 0 1 .76-1.6c-2.67-.3-5.47-1.33-5.47-5.93a4.64 4.64 0 0 1 1.24-3.22 4.32 4.32 0 0 1 .12-3.18s1-.32 3.3 1.23a11.37 11.37 0 0 1 6 0c2.28-1.55 3.29-1.23 3.29-1.23a4.32 4.32 0 0 1 .12 3.18 4.64 4.64 0 0 1 1.23 3.22c0 4.61-2.8 5.63-5.48 5.92a2.86 2.86 0 0 1 .82 2.22l-.01 3.29c0 .32.21.7.82.58A12 12 0 0 0 12 .3"/>
</svg>`

function page(title: string, bodyContent: string, extraStyles: string = '', script: string = '') {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>${title}</title>
    <style>${SHARED_STYLES}${extraStyles}</style>
  </head>
  <body>
    ${bodyContent}
    ${script ? `<script>${script}</script>` : ''}
  </body>
</html>`
}

export function desktopSignInPage(signInEndpoint: string, callbackUrl: string) {
  const extraStyles = `
    .spinner{
      width:20px;height:20px;
      border:2px solid var(--border);border-top-color:var(--primary);
      border-radius:50%;
      animation:spin .6s linear infinite;
    }
    @keyframes spin{to{transform:rotate(360deg)}}
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
  `

  const body = `
    <div class="card" id="root">
      ${LOGO_MARKUP}
      ${GITHUB_ICON}
      <div class="row">
        <div class="spinner"></div>
        <p id="status">Connecting to GitHub...</p>
      </div>
      <button class="retry" onclick="location.reload()">Try again</button>
    </div>
  `

  const script = `
    const root = document.getElementById('root');
    const statusNode = document.getElementById('status');

    async function startSignIn() {
      try {
        const response = await fetch(${JSON.stringify(signInEndpoint)}, {
          method: 'POST',
          credentials: 'include',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            provider: 'github',
            callbackURL: ${JSON.stringify(callbackUrl)},
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
  `

  return page('Reviu — Sign in with GitHub', body, extraStyles, script)
}

export function desktopSignInSuccessPage(deepLinkUrl: string) {
  const extraStyles = `
    .check{
      width:48px;height:48px;
      border-radius:50%;
      background:var(--primary);
      display:flex;align-items:center;justify-content:center;
      animation:pop .3s ease-out;
    }
    @keyframes pop{
      0%{transform:scale(0);opacity:0}
      70%{transform:scale(1.1)}
      100%{transform:scale(1);opacity:1}
    }
    .check svg{width:24px;height:24px;color:#fff}
    .hint{font-size:13px;color:var(--muted);margin-top:-12px}
  `

  const body = `
    <div class="card">
      ${LOGO_MARKUP}
      <div class="check">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="20 6 9 17 4 12"/>
        </svg>
      </div>
      <p id="status">Connected — opening Reviu...</p>
      <p class="hint">You can close this tab.</p>
    </div>
  `

  const script = `
    window.location.replace(${JSON.stringify(deepLinkUrl)});
  `

  return page('Reviu — Connected', body, extraStyles, script)
}

export function desktopSignInErrorPage() {
  const extraStyles = `
    .error-icon{
      width:48px;height:48px;
      border-radius:50%;
      background:#ef4444;
      display:flex;align-items:center;justify-content:center;
    }
    .error-icon svg{width:24px;height:24px;color:#fff}
    #status{color:#ef4444}
  `

  const body = `
    <div class="card">
      ${LOGO_MARKUP}
      <div class="error-icon">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
        </svg>
      </div>
      <p id="status">Sign in failed. Please try again from the app.</p>
    </div>
  `

  return page('Reviu — Sign in failed', body, extraStyles)
}
