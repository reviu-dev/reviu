import { describe, expect, it } from 'vitest'
import { assetUrlFor, AssetValidationError, validateAssetUpload } from '../service.js'

describe('asset upload validation', () => {
  it('accepts supported image content types', () => {
    const bytes = new Uint8Array([1, 2, 3])
    const result = validateAssetUpload({ bytes, contentType: 'image/png' })
    expect(result).toEqual({ bytes, contentType: 'image/png' })
  })

  it('normalizes content type casing', () => {
    const bytes = new Uint8Array([1, 2, 3])
    const result = validateAssetUpload({ bytes, contentType: 'IMAGE/JPEG' })
    expect(result.contentType).toBe('image/jpeg')
  })

  it('rejects missing content type', () => {
    try {
      validateAssetUpload({ bytes: new Uint8Array([1]), contentType: null })
      expect.unreachable('expected validation error')
    }
    catch (error) {
      expect(error).toBeInstanceOf(AssetValidationError)
      expect((error as AssetValidationError).status).toBe(400)
    }
  })

  it('rejects unsupported content types', () => {
    try {
      validateAssetUpload({
        bytes: new Uint8Array([1]),
        contentType: 'application/pdf',
      })
      expect.unreachable('expected validation error')
    }
    catch (error) {
      expect(error).toBeInstanceOf(AssetValidationError)
      expect((error as AssetValidationError).status).toBe(415)
    }
  })

  it('rejects empty payloads', () => {
    try {
      validateAssetUpload({
        bytes: new Uint8Array(),
        contentType: 'image/png',
      })
      expect.unreachable('expected validation error')
    }
    catch (error) {
      expect(error).toBeInstanceOf(AssetValidationError)
      expect((error as AssetValidationError).status).toBe(400)
    }
  })

  it('rejects payloads above the 10MB limit', () => {
    const bytes = new Uint8Array(10 * 1024 * 1024 + 1)
    try {
      validateAssetUpload({ bytes, contentType: 'image/png' })
      expect.unreachable('expected validation error')
    }
    catch (error) {
      expect(error).toBeInstanceOf(AssetValidationError)
      expect((error as AssetValidationError).status).toBe(413)
    }
  })
})

describe('assetUrlFor', () => {
  it('joins a base URL and an asset id without duplicating slashes', () => {
    const url = assetUrlFor('http://localhost:3000/assets/', {
      id: 'abc',
      contentType: 'image/png',
      byteLength: 42,
    })
    expect(url).toBe('http://localhost:3000/assets/abc')
  })

  it('handles base URLs without a trailing slash', () => {
    const url = assetUrlFor('https://assets.example.com', {
      id: 'abc',
      contentType: 'image/png',
      byteLength: 42,
    })
    expect(url).toBe('https://assets.example.com/abc')
  })
})
