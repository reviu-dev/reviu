import type { AssetStorage, StoredAsset, StoredAssetBody } from '../../plugins/assets/types.js'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const assetStorageMock = {
  put: vi.fn<AssetStorage['put']>(),
  get: vi.fn<AssetStorage['get']>(),
}

vi.mock('../../middlewares/auth.js', async () => {
  const { createMiddleware } = await import('hono/factory')

  return {
    authMiddlewarePro: createMiddleware(async (ctx, next) => {
      ctx.set('user', {
        id: 'user-1',
        email: 'user@example.com',
        role: 'user',
        github: {
          accessToken: 'github-token',
          scopes: ['repo'],
        },
      } as any)
      await next()
    }),
  }
})

vi.mock('../../plugins/assets/runtime.js', () => ({
  assetStorage: assetStorageMock,
  assetsBaseUrl: () => 'http://localhost:3000/assets',
}))

async function request(path: string, init?: RequestInit) {
  const { assetsRoutes } = await import('../assets.js')
  return assetsRoutes.request(`http://localhost${path}`, init)
}

function makeStoredAsset(): StoredAsset {
  return {
    id: '00000000-0000-4000-8000-000000000001',
    contentType: 'image/png',
    byteLength: 8,
  }
}

function makeStoredAssetBody(): StoredAssetBody {
  return {
    ...makeStoredAsset(),
    bytes: new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
  }
}

function pngFormData() {
  const form = new FormData()
  const bytes = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10])
  form.append('file', new File([bytes], 'hello.png', { type: 'image/png' }))
  return form
}

describe('assets routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('uploads a PNG and returns the public URL', async () => {
    assetStorageMock.put.mockResolvedValue(makeStoredAsset())

    const response = await request('/upload', {
      method: 'POST',
      body: pngFormData(),
    })

    expect(response.status).toBe(201)
    await expect(response.json()).resolves.toEqual({
      url: 'http://localhost:3000/assets/00000000-0000-4000-8000-000000000001',
    })
    expect(assetStorageMock.put).toHaveBeenCalledWith({
      bytes: expect.any(Uint8Array),
      contentType: 'image/png',
    })
  })

  it('rejects uploads with no file field', async () => {
    const response = await request('/upload', {
      method: 'POST',
      body: new FormData(),
    })

    expect(response.status).toBe(400)
    await expect(response.json()).resolves.toEqual({ error: 'Missing `file` field' })
    expect(assetStorageMock.put).not.toHaveBeenCalled()
  })

  it('rejects unsupported content types', async () => {
    const form = new FormData()
    form.append('file', new File(['<pdf>'], 'paper.pdf', { type: 'application/pdf' }))

    const response = await request('/upload', { method: 'POST', body: form })

    expect(response.status).toBe(415)
    expect(assetStorageMock.put).not.toHaveBeenCalled()
  })

  it('rejects payloads above the size limit', async () => {
    const form = new FormData()
    const bytes = new Uint8Array(10 * 1024 * 1024 + 1)
    form.append('file', new File([bytes], 'big.png', { type: 'image/png' }))

    const response = await request('/upload', { method: 'POST', body: form })

    expect(response.status).toBe(413)
    expect(assetStorageMock.put).not.toHaveBeenCalled()
  })

  it('serves a stored asset with the right headers', async () => {
    assetStorageMock.get.mockResolvedValue(makeStoredAssetBody())

    const response = await request('/00000000-0000-4000-8000-000000000001')

    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toBe('image/png')
    expect(response.headers.get('content-length')).toBe('8')
    expect(response.headers.get('cache-control')).toContain('immutable')
    const buffer = await response.arrayBuffer()
    expect(buffer.byteLength).toBe(8)
  })

  it('returns 404 for unknown ids', async () => {
    assetStorageMock.get.mockResolvedValue(null)

    const response = await request('/does-not-exist')

    expect(response.status).toBe(404)
  })
})
