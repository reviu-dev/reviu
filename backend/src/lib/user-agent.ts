interface ParsedDesktopClient {
  version: string
  platform: string | null
  arch: string | null
}

export function parseDesktopUserAgent(ua: string | undefined): ParsedDesktopClient | null {
  if (!ua || !ua.startsWith('Reviu-Desktop/')) {
    return null
  }

  const rest = ua.slice('Reviu-Desktop/'.length)
  const spaceIndex = rest.indexOf(' ')
  const version = spaceIndex === -1 ? rest : rest.slice(0, spaceIndex)

  if (!version) {
    return null
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
