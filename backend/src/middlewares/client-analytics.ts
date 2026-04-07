import { createMiddleware } from 'hono/factory'
import { clientAnalyticsCollector } from '../plugins/client-analytics/client-analytics-collector.js'

interface ParsedClientInfo {
  version: string
  platform: string | null
  arch: string | null
}

function parseUserAgent(ua: string | undefined): ParsedClientInfo {
  if (!ua || !ua.startsWith('Reviu-Desktop/')) {
    return { version: 'unknown', platform: null, arch: null }
  }

  const rest = ua.slice('Reviu-Desktop/'.length)
  const spaceIndex = rest.indexOf(' ')
  const version = spaceIndex === -1 ? rest : rest.slice(0, spaceIndex)

  if (!version) {
    return { version: 'unknown', platform: null, arch: null }
  }

  let platform: string | null = null
  let arch: string | null = null

  if (spaceIndex !== -1) {
    const parens = rest.slice(spaceIndex + 1)
    const openParen = parens.indexOf('(')
    const closeParen = parens.indexOf(')')
    if (openParen !== -1 && closeParen > openParen) {
      const inner = parens.slice(openParen + 1, closeParen)
      const semicolon = inner.indexOf(';')
      if (semicolon !== -1) {
        platform = inner.slice(0, semicolon).trim() || null
        arch = inner.slice(semicolon + 1).trim() || null
      }
    }
  }

  return { version, platform, arch }
}

const IGNORED_PREFIXES = ['/healthcheck', '/api/auth', '/admin']

const ROUTE_NORMALIZATION_PATTERNS: Array<{ pattern: RegExp, replacement: string }> = [
  // /github/notifications/:threadId/...
  { pattern: /^(\/github\/notifications)\/[^/]+/, replacement: '$1/:threadId' },
  // /github/pr/:id/comments/:commentId/...
  { pattern: /^(\/github\/pr)\/[^/]+(\/comments)\/[^/]+/, replacement: '$1/:id$2/:commentId' },
  // /github/pr/:prId/comments/:commentId/replies
  { pattern: /^(\/github\/pr)\/[^/]+(\/comments\/[^/]+\/replies)/, replacement: '$1/:prId/comments/:commentId/replies' },
  // /github/pr/:id/...
  { pattern: /^(\/github\/pr)\/[^/]+/, replacement: '$1/:id' },
  // /github/repos/:owner/:repo/issues/:issue_number/comments/:comment_id
  { pattern: /^(\/github\/repos)\/[^/]+\/[^/]+(\/issues)\/[^/]+(\/comments)\/[^/]+/, replacement: '$1/:owner/:repo$2/:issue_number$3/:comment_id' },
  // /github/repos/:owner/:repo/issues/:issue_number/...
  { pattern: /^(\/github\/repos)\/[^/]+\/[^/]+(\/issues)\/[^/]+/, replacement: '$1/:owner/:repo$2/:issue_number' },
  // /github/repos/:owner/:repo/trees/:tree_sha
  { pattern: /^(\/github\/repos)\/[^/]+\/[^/]+(\/trees)\/[^/]+/, replacement: '$1/:owner/:repo$2/:tree_sha' },
  // /github/repos/:owner/:repo/...
  { pattern: /^(\/github\/repos)\/[^/]+\/[^/]+/, replacement: '$1/:owner/:repo' },
]

function normalizeRoute(pathname: string): string {
  for (const { pattern, replacement } of ROUTE_NORMALIZATION_PATTERNS) {
    if (pattern.test(pathname)) {
      return pathname.replace(pattern, replacement)
    }
  }
  return pathname
}

export const clientAnalyticsMiddleware = createMiddleware(async (c, next) => {
  await next()

  const url = new URL(c.req.url)
  if (IGNORED_PREFIXES.some(p => url.pathname.startsWith(p))) {
    return
  }

  const ua = c.req.header('user-agent')
  const client = parseUserAgent(ua)

  // user may not be set on unauthenticated routes
  const user = c.get('user' as never) as { id: string } | undefined

  clientAnalyticsCollector.record({
    clientVersion: client.version,
    clientPlatform: client.platform,
    clientArch: client.arch,
    method: c.req.method,
    route: normalizeRoute(url.pathname),
    userId: user?.id ?? null,
  })
})
