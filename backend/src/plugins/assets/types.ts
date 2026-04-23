export interface StoredAsset {
  id: string
  contentType: string
  byteLength: number
}

export interface StoredAssetBody extends StoredAsset {
  bytes: Uint8Array
}

export interface AssetStorage {
  put: (params: {
    bytes: Uint8Array
    contentType: string
  }) => Promise<StoredAsset>
  get: (id: string) => Promise<StoredAssetBody | null>
}

export const SUPPORTED_ASSET_CONTENT_TYPES: readonly string[] = [
  'image/png',
  'image/jpeg',
  'image/gif',
  'image/webp',
]

export const MAX_ASSET_BYTES = 10 * 1024 * 1024 // 10 MB

export function extensionForContentType(contentType: string): string {
  switch (contentType) {
    case 'image/png': return 'png'
    case 'image/jpeg': return 'jpg'
    case 'image/gif': return 'gif'
    case 'image/webp': return 'webp'
    default: return 'bin'
  }
}

export function contentTypeForExtension(extension: string): string | null {
  switch (extension.toLowerCase()) {
    case 'png': return 'image/png'
    case 'jpg':
    case 'jpeg': return 'image/jpeg'
    case 'gif': return 'image/gif'
    case 'webp': return 'image/webp'
    default: return null
  }
}
