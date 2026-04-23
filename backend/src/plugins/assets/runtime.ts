import type { AssetStorage } from './types.js'
import path from 'node:path'
import process from 'node:process'
import { env } from '../../lib/env.js'
import { LocalDiskAssetStorage } from './local-disk-storage.js'
import { S3AssetStorage } from './s3-storage.js'

// Production uses Hetzner Object Storage (S3-compatible). The bucket is
// shared with other Reviu data, so all asset keys live under an `assets/`
// prefix to avoid colliding with unrelated objects.
function createStorage(): AssetStorage {
  if (env.ASSETS_USE_MOCK) {
    return new LocalDiskAssetStorage({
      rootDir: env.ASSETS_MOCK_ROOT ?? path.resolve(process.cwd(), '.assets-mock'),
    })
  }

  return new S3AssetStorage({
    endpoint: env.HETZNER_STORAGE_ENDPOINT,
    region: env.HETZNER_STORAGE_REGION,
    bucket: env.HETZNER_STORAGE_BUCKET,
    accessKeyId: env.HETZNER_STORAGE_ACCESS_KEY,
    secretAccessKey: env.HETZNER_STORAGE_SECRET_KEY,
    keyPrefix: 'assets',
  })
}

export const assetStorage = createStorage()

export function assetsBaseUrl(): string {
  if (env.ASSETS_BASE_URL)
    return env.ASSETS_BASE_URL
  return `${env.BASE_URL.replace(/\/+$/, '')}/assets`
}
