import type { AssetStorage, StoredAsset } from './types.js'
import { MAX_ASSET_BYTES, SUPPORTED_ASSET_CONTENT_TYPES } from './types.js'

export class AssetValidationError extends Error {
  readonly status: 400 | 413 | 415

  constructor(status: 400 | 413 | 415, message: string) {
    super(message)
    this.status = status
  }
}

export function assetUrlFor(baseUrl: string, asset: StoredAsset): string {
  const trimmed = baseUrl.replace(/\/+$/, '')
  return `${trimmed}/${asset.id}`
}

export function validateAssetUpload(params: {
  bytes: Uint8Array
  contentType: string | undefined | null
}): { bytes: Uint8Array, contentType: string } {
  if (!params.contentType) {
    throw new AssetValidationError(400, 'Missing content type')
  }

  const normalizedType = params.contentType.toLowerCase()
  if (!SUPPORTED_ASSET_CONTENT_TYPES.includes(normalizedType)) {
    throw new AssetValidationError(
      415,
      `Unsupported content type: ${params.contentType}`,
    )
  }

  if (params.bytes.byteLength === 0) {
    throw new AssetValidationError(400, 'Empty file payload')
  }

  if (params.bytes.byteLength > MAX_ASSET_BYTES) {
    throw new AssetValidationError(413, 'File exceeds 10MB limit')
  }

  return { bytes: params.bytes, contentType: normalizedType }
}

export async function uploadAsset(
  storage: AssetStorage,
  params: { bytes: Uint8Array, contentType: string | undefined | null },
): Promise<StoredAsset> {
  const validated = validateAssetUpload(params)
  return storage.put(validated)
}
