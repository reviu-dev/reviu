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
const DESKTOP_UPDATE_RELEASE_OWNER = 'joris-gallot'
const DESKTOP_UPDATE_RELEASE_REPO = 'reviu'
const DESKTOP_UPDATE_MANIFEST_FILE_NAME = 'desktop-update.manifest.json'

interface GithubReleaseAssetRef {
  id: number
  name: string
  content_type?: string | null
  size: number
}

interface GithubReleaseRef {
  assets: GithubReleaseAssetRef[]
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

export function normalizeSemver(value: string): string | null {
  return clean(value.trim())
}

function githubHeaders(accept: string) {
  return {
    'Authorization': `Bearer ${env.GITHUB_TOKEN}`,
    'Accept': accept,
    'X-GitHub-Api-Version': '2022-11-28',
  }
}

function releaseTagFromVersion(version: string) {
  return `v${version}`
}

function extractAssetFileName(url: string) {
  try {
    const parsedUrl = new URL(url)
    const fileName = parsedUrl.pathname.split('/').pop()?.trim()
    return fileName || null
  }
  catch {
    return null
  }
}

function buildDesktopUpdateReleaseDownloadUrl(tag: string, fileName: string) {
  return new URL(
    `desktop/update/download/release/${encodeURIComponent(tag)}/${encodeURIComponent(fileName)}`,
    env.BASE_URL.endsWith('/') ? env.BASE_URL : `${env.BASE_URL}/`,
  ).toString()
}

function findReleaseAssetByName(release: GithubReleaseRef, fileName: string) {
  const asset = release.assets.find(candidate => candidate.name === fileName)

  if (!asset) {
    throw new Error(`Asset not found: ${fileName}`)
  }

  return asset
}

async function fetchReleaseAssetData(asset: GithubReleaseAssetRef) {
  const assetRes = await request(
    'GET /repos/{owner}/{repo}/releases/assets/{asset_id}',
    {
      owner: DESKTOP_UPDATE_RELEASE_OWNER,
      repo: DESKTOP_UPDATE_RELEASE_REPO,
      asset_id: asset.id,
      headers: githubHeaders('application/octet-stream'),
    },
  )

  if (!(assetRes.data instanceof ArrayBuffer)) {
    throw new TypeError('Unexpected asset response type')
  }

  return {
    fileName: asset.name,
    contentType: asset.content_type || 'application/octet-stream',
    size: asset.size,
    data: assetRes.data,
  }
}

function resolveMatchingDesktopArtifact(
  manifest: DesktopUpdateManifest,
  input: Pick<DesktopUpdateCheckInput, 'platform' | 'arch'>,
) {
  const matchedArtifact = manifest.artifacts.find(candidate =>
    candidate.platform === input.platform
    && candidate.arch === input.arch,
  )

  if (!matchedArtifact) {
    throw new Error(
      `No desktop update artifact for platform=${input.platform} arch=${input.arch}`,
    )
  }

  return matchedArtifact
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

async function fetchLatestReleaseMetadata() {
  return request(
    'GET /repos/{owner}/{repo}/releases/latest',
    {
      owner: DESKTOP_UPDATE_RELEASE_OWNER,
      repo: DESKTOP_UPDATE_RELEASE_REPO,
      headers: githubHeaders('application/vnd.github+json'),
    },
  )
}

async function fetchReleaseMetadataByTag(tag: string) {
  return request(
    'GET /repos/{owner}/{repo}/releases/tags/{tag}',
    {
      owner: DESKTOP_UPDATE_RELEASE_OWNER,
      repo: DESKTOP_UPDATE_RELEASE_REPO,
      tag,
      headers: githubHeaders('application/vnd.github+json'),
    },
  )
}

async function fetchManifestFromRelease(release: GithubReleaseRef): Promise<DesktopUpdateManifest> {
  const manifestAsset = findReleaseAssetByName(release, DESKTOP_UPDATE_MANIFEST_FILE_NAME)
  const manifestData = await fetchReleaseAssetData(manifestAsset)
  const text = Buffer.from(manifestData.data).toString('utf-8')
  const json = JSON.parse(text)

  return parseDesktopUpdateManifest(json)
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
  const releaseRes = await fetchLatestReleaseMetadata()
  return fetchManifestFromRelease(releaseRes.data)
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
    const matchedArtifact = resolveMatchingDesktopArtifact(manifest, input)
    const assetFileName = extractAssetFileName(matchedArtifact.url)

    if (!assetFileName) {
      throw new Error(`Invalid desktop update artifact url: ${matchedArtifact.url}`)
    }

    artifact = {
      url: buildDesktopUpdateReleaseDownloadUrl(
        releaseTagFromVersion(manifest.version),
        assetFileName,
      ),
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

export interface DesktopChangelogEntry {
  version: string
  publishedAt: string
  body: string
}

export async function fetchChangelog(): Promise<DesktopChangelogEntry[]> {
  const res = await request(
    'GET /repos/{owner}/{repo}/releases',
    {
      owner: DESKTOP_UPDATE_RELEASE_OWNER,
      repo: DESKTOP_UPDATE_RELEASE_REPO,
      per_page: 50,
      headers: githubHeaders('application/vnd.github+json'),
    },
  )

  return res.data
    .filter((release: { draft?: boolean, body?: string | null }) => !release.draft && release.body)
    .map((release: { tag_name: string, published_at?: string | null, body?: string | null }) => ({
      version: release.tag_name.replace(/^v/, ''),
      publishedAt: release.published_at ?? '',
      body: release.body ?? '',
    }))
    .sort((a: DesktopChangelogEntry, b: DesktopChangelogEntry) =>
      new Date(b.publishedAt).getTime() - new Date(a.publishedAt).getTime(),
    )
}

export async function downloadDesktopUpdateReleaseAsset(tag: string, fileName: string) {
  const releaseRes = await fetchReleaseMetadataByTag(tag)
  const asset = findReleaseAssetByName(releaseRes.data, fileName)
  return fetchReleaseAssetData(asset)
}

export async function downloadLatestDesktopUpdateAsset(
  input: Pick<DesktopUpdateCheckInput, 'platform' | 'arch'>,
) {
  const releaseRes = await fetchLatestReleaseMetadata()
  const manifest = await fetchManifestFromRelease(releaseRes.data)
  const matchedArtifact = resolveMatchingDesktopArtifact(manifest, input)
  const assetFileName = extractAssetFileName(matchedArtifact.url)

  if (!assetFileName) {
    throw new Error(`Invalid desktop update artifact url: ${matchedArtifact.url}`)
  }

  const asset = findReleaseAssetByName(releaseRes.data, assetFileName)
  return fetchReleaseAssetData(asset)
}

export function clearDesktopUpdateManifestCache() {
  desktopUpdateManifestCache = null
}
