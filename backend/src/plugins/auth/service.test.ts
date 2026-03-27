import type { AuthCodeStore } from './service.js'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AUTH_CODE_TTL_MS, createAuthCodeService } from './service.js'

class MemoryAuthCodeStore implements AuthCodeStore {
  private readonly entries = new Map<string, { value: string, expiresAt: number }>()

  async set(key: string, value: string, ttlMs: number): Promise<void> {
    this.entries.set(key, {
      value,
      expiresAt: Date.now() + ttlMs,
    })
  }

  async getDel(key: string): Promise<string | null> {
    const entry = this.entries.get(key)

    if (!entry) {
      return null
    }

    this.entries.delete(key)

    if (Date.now() > entry.expiresAt) {
      return null
    }

    return entry.value
  }
}

describe('auth code service', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('issues a single-use code that resolves to the original token', async () => {
    const service = createAuthCodeService(new MemoryAuthCodeStore())

    const code = await service.issueAuthCode('session-token')

    expect(code).toMatch(/^[a-f0-9]{64}$/)
    await expect(service.consumeAuthCode(code)).resolves.toBe('session-token')
    await expect(service.consumeAuthCode(code)).resolves.toBeNull()
  })

  it('expires auth codes after the configured ttl', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-03-27T10:00:00Z'))

    const service = createAuthCodeService(new MemoryAuthCodeStore())
    const code = await service.issueAuthCode('session-token')

    vi.advanceTimersByTime(AUTH_CODE_TTL_MS + 1)

    await expect(service.consumeAuthCode(code)).resolves.toBeNull()
  })
})
