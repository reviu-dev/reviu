import type { AssetStorage, StoredAsset, StoredAssetBody } from './types.js'
import { randomUUID } from 'node:crypto'
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { contentTypeForExtension, extensionForContentType } from './types.js'

interface LocalDiskAssetStorageOptions {
  rootDir: string
}

export class LocalDiskAssetStorage implements AssetStorage {
  private readonly rootDir: string
  private rootEnsured = false

  constructor(options: LocalDiskAssetStorageOptions) {
    this.rootDir = options.rootDir
  }

  private async ensureRoot(): Promise<void> {
    if (this.rootEnsured)
      return
    await mkdir(this.rootDir, { recursive: true })
    this.rootEnsured = true
  }

  async put(params: { bytes: Uint8Array, contentType: string }): Promise<StoredAsset> {
    await this.ensureRoot()
    const id = randomUUID()
    const extension = extensionForContentType(params.contentType)
    const filename = `${id}.${extension}`
    await writeFile(path.join(this.rootDir, filename), params.bytes)
    return {
      id,
      contentType: params.contentType,
      byteLength: params.bytes.byteLength,
    }
  }

  async get(id: string): Promise<StoredAssetBody | null> {
    await this.ensureRoot()
    // Reject anything that could escape the root (UUIDs only contain [0-9a-f-]).
    if (!/^[a-f0-9-]{36}$/i.test(id))
      return null

    const entries = await readdir(this.rootDir)
    const match = entries.find(entry => entry.startsWith(`${id}.`))
    if (!match)
      return null

    const extension = match.slice(id.length + 1)
    const contentType = contentTypeForExtension(extension)
    if (!contentType)
      return null

    const bytes = await readFile(path.join(this.rootDir, match))
    return {
      id,
      contentType,
      byteLength: bytes.byteLength,
      bytes: new Uint8Array(bytes),
    }
  }
}
