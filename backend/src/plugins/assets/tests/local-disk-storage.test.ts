import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { LocalDiskAssetStorage } from '../local-disk-storage.js'

describe('localDiskAssetStorage', () => {
  let rootDir: string
  let storage: LocalDiskAssetStorage

  beforeEach(async () => {
    rootDir = await mkdtemp(path.join(tmpdir(), 'reviu-assets-'))
    storage = new LocalDiskAssetStorage({ rootDir })
  })

  afterEach(async () => {
    await rm(rootDir, { recursive: true, force: true })
  })

  it('round-trips bytes and content type through put + get', async () => {
    const bytes = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10])
    const stored = await storage.put({ bytes, contentType: 'image/png' })

    expect(stored.id).toMatch(/^[a-f0-9-]{36}$/)
    expect(stored.contentType).toBe('image/png')
    expect(stored.byteLength).toBe(bytes.byteLength)

    const fetched = await storage.get(stored.id)
    expect(fetched).not.toBeNull()
    expect(fetched?.contentType).toBe('image/png')
    expect(fetched?.byteLength).toBe(bytes.byteLength)
    expect(Array.from(fetched!.bytes)).toEqual(Array.from(bytes))
  })

  it('returns null for unknown ids', async () => {
    const missing = await storage.get('00000000-0000-4000-8000-000000000000')
    expect(missing).toBeNull()
  })

  it('rejects ids that look like path traversal attempts', async () => {
    const escaped = await storage.get('../secret')
    expect(escaped).toBeNull()
  })
})
