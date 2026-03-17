import { Buffer } from 'node:buffer'
import { readFile } from 'node:fs/promises'
import { request } from '@octokit/request'
import { clean, lt } from 'semver'
import { z } from 'zod'
import { env } from '../../lib/env.js'

const desktopPlatformSchema = z.enum(['macos', 'linux', 'windows'])
const desktopArchSchema = z.enum(['x86_64', 'aarch64'])

const desktopUpdateArtifactSchema = z.object({
  platform: desktopPlatformSchema,
  arch: desktopArchSchema,
  url: z.url(),
  sha256: z.string(),
  size: z.number().int().positive(),
})

const desktopUpdateManifestSchema = z.object({
  version: z.string().trim(),
  minimumSupportedVersion: z.string().trim(),
  releaseNotesUrl: z.url(),
  artifacts: z.array(desktopUpdateArtifactSchema).min(1),
})

const DESKTOP_UPDATE_MANIFEST_CACHE_TTL_MS = 5 * 60 * 1000

interface CachedDesktopUpdateManifest {
  value: DesktopUpdateManifest
  expiresAt: number
}

export type DesktopPlatform = z.infer<typeof desktopPlatformSchema>
export type DesktopArch = z.infer<typeof desktopArchSchema>

export interface DesktopUpdateArtifact {
  platform: DesktopPlatform
  arch: DesktopArch
  url: string
  sha256: string
  size: number
}

export interface DesktopUpdateManifest {
  version: string
  minimumSupportedVersion: string
  releaseNotesUrl: string
  artifacts: DesktopUpdateArtifact[]
}

export interface DesktopUpdateCheckInput {
  currentVersion: string
  platform: DesktopPlatform
  arch: DesktopArch
}

export interface DesktopUpdateCheckResult {
  updateAvailable: boolean
  forceUpdate: boolean
  currentVersion: string
  latestVersion: string
  minimumSupportedVersion: string
  releaseNotesUrl: string
  artifact: {
    url: string
    sha256: string
    size: number
  } | null
}

let desktopUpdateManifestCache: CachedDesktopUpdateManifest | null = null

export function normalizeSemver(value: string): string | null {
  return clean(value.trim())
}

export function parseDesktopUpdateManifest(payload: unknown): DesktopUpdateManifest {
  const parsedManifest = desktopUpdateManifestSchema.safeParse(payload)

  if (!parsedManifest.success) {
    throw new Error('Invalid desktop update manifest payload')
  }

  const version = normalizeSemver(parsedManifest.data.version)
  if (!version) {
    throw new Error('Invalid desktop update manifest version')
  }

  const minimumSupportedVersion = normalizeSemver(
    parsedManifest.data.minimumSupportedVersion,
  )
  if (!minimumSupportedVersion) {
    throw new Error('Invalid desktop update minimum supported version')
  }

  return {
    version,
    minimumSupportedVersion,
    releaseNotesUrl: parsedManifest.data.releaseNotesUrl,
    artifacts: parsedManifest.data.artifacts.map(artifact => ({
      platform: artifact.platform,
      arch: artifact.arch,
      url: artifact.url,
      sha256: artifact.sha256.toLowerCase(),
      size: artifact.size,
    })),
  } satisfies DesktopUpdateManifest
}

async function loadDevelopmentManifestFromFile(): Promise<DesktopUpdateManifest> {
  const manifestPath = new URL('../../dev/desktop-update.manifest.json', import.meta.url)

  let contents: string
  try {
    contents = await readFile(manifestPath, 'utf8')
  }
  catch (error) {
    throw new Error(
      `Desktop update development manifest is missing or unreadable: ${(error as Error).message}`,
    )
  }

  let parsed: unknown
  try {
    parsed = JSON.parse(contents)
  }
  catch (error) {
    throw new Error(
      `Desktop update development manifest contains invalid JSON: ${(error as Error).message}`,
    )
  }

  return parseDesktopUpdateManifest(parsed)
}

async function fetchProductionManifestFromRemote(): Promise<DesktopUpdateManifest> {
  const owner = 'joris-gallot'
  const repo = 'reviu'
  const fileName = 'desktop-update.manifest.json'

  const releaseRes = await request(
    'GET /repos/{owner}/{repo}/releases/latest',
    {
      owner,
      repo,
      headers: {
        'Authorization': `Bearer ${env.GITHUB_TOKEN}`,
        'Accept': 'application/vnd.github+json',
        'X-GitHub-Api-Version': '2022-11-28',
      },
    },
  )

  const asset = releaseRes.data.assets.find(a => a.name === fileName)

  if (!asset) {
    throw new Error(`Asset not found: ${fileName}`)
  }

  const assetRes = await request(
    'GET /repos/{owner}/{repo}/releases/assets/{asset_id}',
    {
      owner,
      repo,
      asset_id: asset.id,
      headers: {
        'Authorization': `Bearer ${env.GITHUB_TOKEN}`,
        'Accept': 'application/octet-stream',
        'X-GitHub-Api-Version': '2022-11-28',
      },
    },
  )

  if (!(assetRes.data instanceof ArrayBuffer)) {
    throw new TypeError('Unexpected asset response type')
  }

  const text = Buffer.from(assetRes.data).toString('utf-8')
  const json = JSON.parse(text)

  return parseDesktopUpdateManifest(json)
}

export async function fetchDesktopUpdateManifest() {
  if (env.NODE_ENV === 'development') {
    // Dev is intentionally fail-fast and always re-reads the local file.
    return loadDevelopmentManifestFromFile()
  }

  const now = Date.now()
  if (
    desktopUpdateManifestCache
    && desktopUpdateManifestCache.expiresAt > now
  ) {
    return desktopUpdateManifestCache.value
  }

  const manifest = await fetchProductionManifestFromRemote()
  desktopUpdateManifestCache = {
    value: manifest,
    expiresAt: now + DESKTOP_UPDATE_MANIFEST_CACHE_TTL_MS,
  }
  return manifest
}

export function resolveDesktopUpdateCheck(
  manifest: DesktopUpdateManifest,
  input: DesktopUpdateCheckInput,
): DesktopUpdateCheckResult {
  const current = normalizeSemver(input.currentVersion)
  if (!current) {
    throw new Error(`Invalid currentVersion: ${input.currentVersion}`)
  }

  const updateAvailable = lt(current, manifest.version)
  const forceUpdate = lt(current, manifest.minimumSupportedVersion)

  let artifact: DesktopUpdateCheckResult['artifact'] = null
  if (updateAvailable) {
    const matchedArtifact = manifest.artifacts.find(candidate =>
      candidate.platform === input.platform
      && candidate.arch === input.arch,
    )

    if (!matchedArtifact) {
      throw new Error(
        `No desktop update artifact for platform=${input.platform} arch=${input.arch}`,
      )
    }

    artifact = {
      url: matchedArtifact.url,
      sha256: matchedArtifact.sha256,
      size: matchedArtifact.size,
    }
  }

  return {
    updateAvailable,
    forceUpdate,
    currentVersion: current,
    latestVersion: manifest.version,
    minimumSupportedVersion: manifest.minimumSupportedVersion,
    releaseNotesUrl: manifest.releaseNotesUrl,
    artifact,
  } satisfies DesktopUpdateCheckResult
}

export async function checkDesktopUpdate(
  input: DesktopUpdateCheckInput,
) {
  const manifest = await fetchDesktopUpdateManifest()
  return resolveDesktopUpdateCheck(manifest, input)
}

export function clearDesktopUpdateManifestCache() {
  desktopUpdateManifestCache = null
}
