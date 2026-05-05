import { Buffer } from 'node:buffer'
import { createCipheriv, createDecipheriv, createHash, randomBytes } from 'node:crypto'
import { env } from '../../lib/env.js'

const ALGORITHM = 'aes-256-gcm'
const IV_BYTES = 12

function encryptionSecret() {
  const secret = env.AI_CREDENTIALS_SECRET
  if (!secret) {
    throw Object.assign(new Error('AI credential encryption is not configured.'), { status: 503 })
  }

  return createHash('sha256').update(secret).digest()
}

export function encryptSecret(value: string) {
  const iv = randomBytes(IV_BYTES)
  const cipher = createCipheriv(ALGORITHM, encryptionSecret(), iv)
  const encrypted = Buffer.concat([cipher.update(value, 'utf8'), cipher.final()])
  const tag = cipher.getAuthTag()

  return [
    'v1',
    iv.toString('base64url'),
    tag.toString('base64url'),
    encrypted.toString('base64url'),
  ].join(':')
}

export function decryptSecret(value: string) {
  const [version, ivEncoded, tagEncoded, encryptedEncoded] = value.split(':')
  if (version !== 'v1' || !ivEncoded || !tagEncoded || !encryptedEncoded) {
    throw new Error('Invalid encrypted secret format.')
  }

  const decipher = createDecipheriv(
    ALGORITHM,
    encryptionSecret(),
    Buffer.from(ivEncoded, 'base64url'),
  )
  decipher.setAuthTag(Buffer.from(tagEncoded, 'base64url'))

  return Buffer.concat([
    decipher.update(Buffer.from(encryptedEncoded, 'base64url')),
    decipher.final(),
  ]).toString('utf8')
}

export function apiKeyHint(apiKey: string) {
  const trimmed = apiKey.trim()
  if (trimmed.length <= 8) {
    return '****'
  }

  return `${trimmed.slice(0, 4)}...${trimmed.slice(-4)}`
}
