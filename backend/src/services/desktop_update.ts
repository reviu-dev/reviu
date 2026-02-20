import { readFile } from 'node:fs/promises'
import { z } from 'zod'

import { env } from '../lib/env.js'

const semverSchema = z.string().trim().regex(/^v?\d+\.\d+\.\d+$/)
const desktopPlatformSchema = z.enum(['macos', 'linux', 'windows'])
const desktopArchSchema = z.enum(['x86_64', 'aarch64'])
const sha256Schema = z.string().trim().regex(/^[a-f0-9]{64}$/i)

const desktopUpdateArtifactSchema = z.object({
  platform: desktopPlatformSchema,
  arch: desktopArchSchema,
  url: z.url(),
  sha256: sha256Schema,
  size: z.number().int().positive(),
})

const desktopUpdateManifestSchema = z.object({
  version: semverSchema,
  minimumSupportedVersion: semverSchema,
  releaseNotesUrl: z.url(),
  artifacts: z.array(desktopUpdateArtifactSchema).min(1),
})

const DESKTOP_UPDATE_MANIFEST_CACHE_TTL_MS = 5 * 60 * 1000

interface ParsedSemver {
  major: number
  minor: number
  patch: number
  normalized: string
}

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

export function parseSemver(value: string): ParsedSemver | null {
  const parsed = semverSchema.safeParse(value)
  if (!parsed.success) {
    return null
  }

  const normalized = parsed.data.startsWith('v')
    ? parsed.data.slice(1)
    : parsed.data
  const [major, minor, patch] = normalized.split('.').map(Number)
  return {
    major,
    minor,
    patch,
    normalized: `${major}.${minor}.${patch}`,
  }
}

export function compareSemver(left: ParsedSemver, right: ParsedSemver) {
  if (left.major !== right.major) {
    return Math.sign(left.major - right.major)
  }
  if (left.minor !== right.minor) {
    return Math.sign(left.minor - right.minor)
  }
  return Math.sign(left.patch - right.patch)
}

export function normalizeSemver(value: string) {
  return parseSemver(value)?.normalized ?? null
}

async function parseManifestPayload(payload: unknown): Promise<DesktopUpdateManifest> {
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

  return parseManifestPayload(parsed)
}

async function fetchProductionManifestFromRemote(): Promise<DesktopUpdateManifest> {
  if (!env.DESKTOP_UPDATE_MANIFEST_URL) {
    throw new Error('Missing DESKTOP_UPDATE_MANIFEST_URL in production')
  }

  const response = await fetch(env.DESKTOP_UPDATE_MANIFEST_URL)
  if (!response.ok) {
    throw new Error(
      `Desktop update manifest fetch failed with status ${response.status}`,
    )
  }

  const manifestResponse = await response.json()
  return parseManifestPayload(manifestResponse)
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

export async function checkDesktopUpdate(
  input: DesktopUpdateCheckInput,
) {
  const normalizedCurrentVersion = normalizeSemver(input.currentVersion)
  if (!normalizedCurrentVersion) {
    throw new Error('Invalid current version')
  }

  const manifest = await fetchDesktopUpdateManifest()

  const current = parseSemver(normalizedCurrentVersion)!
  const latest = parseSemver(manifest.version)!
  const minimumSupported = parseSemver(manifest.minimumSupportedVersion)!
  const updateAvailable = compareSemver(current, latest) < 0
  const forceUpdate = compareSemver(current, minimumSupported) < 0

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
    currentVersion: normalizedCurrentVersion,
    latestVersion: manifest.version,
    minimumSupportedVersion: manifest.minimumSupportedVersion,
    releaseNotesUrl: manifest.releaseNotesUrl,
    artifact,
  } satisfies DesktopUpdateCheckResult
}

export function clearDesktopUpdateManifestCache() {
  desktopUpdateManifestCache = null
}
