import crypto from 'node:crypto'

const AUTH_CODE_TTL_MS = 5 * 60 * 1000 // 5 minutes
const authCodeStore = new Map<string, { token: string, expiresAt: number }>()

export function issueAuthCode(token: string) {
  const code = crypto.randomBytes(32).toString('hex')
  authCodeStore.set(code, { token, expiresAt: Date.now() + AUTH_CODE_TTL_MS })
  return code
}

export function consumeAuthCode(code: string) {
  const entry = authCodeStore.get(code)
  if (!entry) {
    return null
  }

  if (Date.now() > entry.expiresAt) {
    authCodeStore.delete(code)
    return null
  }

  authCodeStore.delete(code)
  return entry.token
}
