import type { AssetStorage, StoredAsset, StoredAssetBody } from './types.js'
import { randomUUID } from 'node:crypto'
import { GetObjectCommand, PutObjectCommand, S3Client } from '@aws-sdk/client-s3'

interface S3AssetStorageOptions {
  endpoint: string
  region: string
  bucket: string
  accessKeyId: string
  secretAccessKey: string
  /**
   * Key prefix applied to every stored object so the bucket can be shared
   * with other data (e.g. `assets/` for a bucket that also hosts unrelated
   * directories). Leading/trailing slashes are normalized.
   */
  keyPrefix?: string
}

export class S3AssetStorage implements AssetStorage {
  private readonly client: S3Client
  private readonly bucket: string
  private readonly keyPrefix: string

  constructor(options: S3AssetStorageOptions) {
    this.bucket = options.bucket
    const trimmedPrefix = options.keyPrefix?.replace(/^\/+|\/+$/g, '') ?? ''
    this.keyPrefix = trimmedPrefix ? `${trimmedPrefix}/` : ''
    this.client = new S3Client({
      endpoint: options.endpoint,
      region: options.region,
      credentials: {
        accessKeyId: options.accessKeyId,
        secretAccessKey: options.secretAccessKey,
      },
      // Hetzner (and most self-hosted S3-compatible providers) expect
      // path-style URLs rather than virtual-hosted-style.
      forcePathStyle: true,
    })
  }

  // The content-type is preserved as S3 object metadata (`ContentType`) and
  // retrieved on GET, so the stored key is just the UUID — no extension
  // mapping to maintain and a single round trip on every fetch.
  private keyFor(id: string): string {
    return `${this.keyPrefix}${id}`
  }

  async put(params: { bytes: Uint8Array, contentType: string }): Promise<StoredAsset> {
    const id = randomUUID()

    await this.client.send(new PutObjectCommand({
      Bucket: this.bucket,
      Key: this.keyFor(id),
      Body: params.bytes,
      ContentType: params.contentType,
      CacheControl: 'public, max-age=31536000, immutable',
    }))

    return {
      id,
      contentType: params.contentType,
      byteLength: params.bytes.byteLength,
    }
  }

  async get(id: string): Promise<StoredAssetBody | null> {
    if (!/^[a-f0-9-]{36}$/i.test(id))
      return null

    try {
      const response = await this.client.send(new GetObjectCommand({
        Bucket: this.bucket,
        Key: this.keyFor(id),
      }))
      if (!response.Body)
        return null
      const bytes = new Uint8Array(await response.Body.transformToByteArray())
      const contentType = response.ContentType?.toLowerCase() ?? 'application/octet-stream'
      return {
        id,
        contentType,
        byteLength: bytes.byteLength,
        bytes,
      }
    }
    catch (error) {
      if (isNoSuchKeyError(error))
        return null
      throw error
    }
  }
}

function isNoSuchKeyError(error: unknown): boolean {
  if (!error || typeof error !== 'object')
    return false
  const name = 'name' in error ? String((error as { name: unknown }).name) : ''
  const code = '$metadata' in error && error.$metadata && typeof error.$metadata === 'object'
    && 'httpStatusCode' in error.$metadata
    ? (error.$metadata as { httpStatusCode?: number }).httpStatusCode
    : undefined
  return name === 'NoSuchKey' || name === 'NotFound' || code === 404
}
