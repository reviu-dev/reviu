import type { GithubCacheStore } from './github-cache.js'

const GITHUB_PUBLIC_REPOSITORY_VISIBILITY_TTL_MS = 2 * 60_000

interface GithubRepositoryVisibilityEntry {
  expiresAt: number
}

function buildRepositoryVisibilityKey(owner: string, repo: string) {
  return `gh:repo-visibility:public:${owner.toLowerCase()}/${repo.toLowerCase()}`
}

function parseVisibilityEntry(value: string | null) {
  if (!value) {
    return null
  }

  try {
    return JSON.parse(value) as GithubRepositoryVisibilityEntry
  }
  catch {
    return null
  }
}

export function createGithubRepositoryVisibilityStore(
  {
    store,
    now = () => Date.now(),
    ttlMs = GITHUB_PUBLIC_REPOSITORY_VISIBILITY_TTL_MS,
  }: {
    store: GithubCacheStore
    now?: () => number
    ttlMs?: number
  },
) {
  return new GithubRepositoryVisibilityStore(store, now, ttlMs)
}

export class GithubRepositoryVisibilityStore {
  constructor(
    private readonly store: GithubCacheStore,
    private readonly now: () => number,
    private readonly ttlMs: number,
  ) {}

  async isKnownPublic(owner: string, repo: string) {
    const key = buildRepositoryVisibilityKey(owner, repo)
    const entry = parseVisibilityEntry(await this.store.get(key))

    if (!entry) {
      return false
    }

    if (entry.expiresAt <= this.now()) {
      await this.store.del([key])
      return false
    }

    return true
  }

  async markPublic(owner: string, repo: string) {
    const key = buildRepositoryVisibilityKey(owner, repo)
    await this.store.set(key, JSON.stringify({
      expiresAt: this.now() + this.ttlMs,
    } satisfies GithubRepositoryVisibilityEntry))
  }

  async clear(owner: string, repo: string) {
    await this.store.del([buildRepositoryVisibilityKey(owner, repo)])
  }
}
