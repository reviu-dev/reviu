import { z } from 'zod'
import { env } from '../lib/env.js'

const semverSchema = z.string().trim().regex(/^v?\d+\.\d+\.\d+$/)

const desktopUpdateManifestSchema = z.object({
  version: semverSchema,
  downloadUrl: z.url(),
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

export interface DesktopUpdateManifest {
  version: string
  downloadUrl: string
}

export interface DesktopUpdateCheckResult {
  updateAvailable: boolean
  currentVersion: string
  latestVersion: string
  downloadUrl: string
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

export async function fetchDesktopUpdateManifest(
  manifestUrl: string,
) {
  const now = Date.now()

  if (
    desktopUpdateManifestCache
    && desktopUpdateManifestCache.expiresAt > now
  ) {
    return desktopUpdateManifestCache.value
  }

  // In development, we return a fixed manifest to avoid issues with fetching/parsing the manifest during development.
  let manifestResponse = {
    version: '0.4.0',
    downloadUrl: 'https://example.com/download',
  }

  if (env.NODE_ENV === 'production') {
    const response = await fetch(manifestUrl)

    if (!response.ok) {
      throw new Error(
        `Desktop update manifest fetch failed with status ${response.status}`,
      )
    }

    manifestResponse = await response.json()
  }

  const parsedManifest = desktopUpdateManifestSchema.safeParse(manifestResponse)
  if (!parsedManifest.success) {
    throw new Error('Invalid desktop update manifest payload')
  }

  const version = normalizeSemver(parsedManifest.data.version)
  if (!version) {
    throw new Error('Invalid desktop update manifest version')
  }

  const manifest = {
    version,
    downloadUrl: parsedManifest.data.downloadUrl,
  } satisfies DesktopUpdateManifest

  desktopUpdateManifestCache = {
    value: manifest,
    expiresAt: now + DESKTOP_UPDATE_MANIFEST_CACHE_TTL_MS,
  }

  return manifest
}

export async function checkDesktopUpdate(
  currentVersion: string,
) {
  const normalizedCurrentVersion = normalizeSemver(currentVersion)

  if (!normalizedCurrentVersion) {
    throw new Error('Invalid current version')
  }

  const manifest = await fetchDesktopUpdateManifest(env.DESKTOP_UPDATE_MANIFEST_URL)

  const current = parseSemver(normalizedCurrentVersion)!
  const latest = parseSemver(manifest.version)!
  const updateAvailable = compareSemver(current, latest) < 0

  return {
    updateAvailable,
    currentVersion: normalizedCurrentVersion,
    latestVersion: manifest.version,
    downloadUrl: manifest.downloadUrl,
  } satisfies DesktopUpdateCheckResult
}

export function clearDesktopUpdateManifestCache() {
  desktopUpdateManifestCache = null
}
