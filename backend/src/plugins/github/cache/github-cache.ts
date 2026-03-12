import { logger } from '../../../lib/logger.js'

export type GithubCacheScope = 'viewer' | 'installation' | 'public'
export type GithubCacheStatus = 'hit' | 'miss' | 'stale'

export interface GithubCacheEntry<T> {
  payload: T
  tags: string[]
  fetchedAt: number
  freshUntil: number
  staleUntil: number
}

export interface GithubCacheStore {
  get: (key: string) => Promise<string | null>
  set: (key: string, value: string) => Promise<void>
  del: (keys: string[]) => Promise<void>
  addToSet: (key: string, members: string[]) => Promise<void>
  removeFromSet: (key: string, members: string[]) => Promise<void>
  getSetMembers: (key: string) => Promise<string[]>
  setIfNotExists: (key: string, value: string, ttlMs: number) => Promise<boolean>
  releaseLock: (key: string, value: string) => Promise<void>
}

export interface GithubCacheGetOrLoadOptions<T> {
  scope: GithubCacheScope
  scopeId?: string
  resourceKey: string
  ttlMs: number
  staleMs: number
  lockTtlMs?: number
  waitForRefreshMs?: number
  tags?: string[]
  load: () => Promise<T>
}

export interface GithubCacheLoadResult<T> {
  payload: T
  cacheStatus: GithubCacheStatus
}

interface MemoryValue {
  value: string
  expiresAt: number | null
}

export class MemoryGithubCacheStore implements GithubCacheStore {
  private readonly values = new Map<string, MemoryValue>()
  private readonly sets = new Map<string, Set<string>>()

  async get(key: string): Promise<string | null> {
    const entry = this.values.get(key)
    if (!entry) {
      return null
    }

    if (entry.expiresAt != null && entry.expiresAt <= Date.now()) {
      this.values.delete(key)
      return null
    }

    return entry.value
  }

  async set(key: string, value: string): Promise<void> {
    this.values.set(key, { value, expiresAt: null })
  }

  async del(keys: string[]): Promise<void> {
    for (const key of keys) {
      this.values.delete(key)
      this.sets.delete(key)
    }
  }

  async addToSet(key: string, members: string[]): Promise<void> {
    if (members.length === 0) {
      return
    }

    const values = this.sets.get(key) ?? new Set<string>()
    for (const member of members) {
      values.add(member)
    }
    this.sets.set(key, values)
  }

  async removeFromSet(key: string, members: string[]): Promise<void> {
    const values = this.sets.get(key)
    if (!values) {
      return
    }

    for (const member of members) {
      values.delete(member)
    }

    if (values.size === 0) {
      this.sets.delete(key)
      return
    }

    this.sets.set(key, values)
  }

  async getSetMembers(key: string): Promise<string[]> {
    return [...(this.sets.get(key) ?? new Set<string>())]
  }

  async setIfNotExists(key: string, value: string, ttlMs: number): Promise<boolean> {
    const currentValue = await this.get(key)
    if (currentValue != null) {
      return false
    }

    this.values.set(key, { value, expiresAt: Date.now() + ttlMs })
    return true
  }

  async releaseLock(key: string, value: string): Promise<void> {
    const currentValue = await this.get(key)
    if (currentValue === value) {
      this.values.delete(key)
    }
  }
}

export function createGithubCache(
  {
    store,
    now = () => Date.now(),
  }: {
    store: GithubCacheStore
    now?: () => number
  },
) {
  return new GithubCacheManager(store, now)
}

function buildCacheKey(scope: GithubCacheScope, resourceKey: string, scopeId?: string) {
  if (scope === 'public') {
    return `gh:cache:public:${resourceKey}`
  }

  if (!scopeId) {
    throw new Error(`Missing scope id for ${scope} cache entry`)
  }

  return `gh:cache:${scope}:${scopeId}:${resourceKey}`
}

function buildLockKey(cacheKey: string) {
  return `gh:lock:${cacheKey}`
}

function buildTagKey(tag: string) {
  return `gh:tag:${tag}`
}

function sleep(ms: number) {
  return new Promise(resolve => setTimeout(resolve, ms))
}

function parseCacheEntry<T>(rawValue: string | null): GithubCacheEntry<T> | null {
  if (!rawValue) {
    return null
  }

  try {
    return JSON.parse(rawValue) as GithubCacheEntry<T>
  }
  catch {
    return null
  }
}

class GithubCacheManager {
  private readonly inflight = new Map<string, Promise<GithubCacheLoadResult<unknown>>>()
  private readonly backgroundRefreshes = new Map<string, Promise<void>>()

  constructor(
    private readonly store: GithubCacheStore,
    private readonly now: () => number,
  ) {}

  async getOrLoad<T>(options: GithubCacheGetOrLoadOptions<T>): Promise<GithubCacheLoadResult<T>> {
    const cacheKey = buildCacheKey(options.scope, options.resourceKey, options.scopeId)
    const inflight = this.inflight.get(cacheKey) as Promise<GithubCacheLoadResult<T>> | undefined
    if (inflight) {
      const result = await inflight
      this.logCacheResolution(options, cacheKey, result.cacheStatus)
      return result
    }

    const task = this.load(cacheKey, options)
    this.inflight.set(cacheKey, task as Promise<GithubCacheLoadResult<unknown>>)

    try {
      const result = await task
      this.logCacheResolution(options, cacheKey, result.cacheStatus)
      return result
    }
    finally {
      this.inflight.delete(cacheKey)
    }
  }

  async invalidateTags(tags: string[]): Promise<void> {
    const keys = new Set<string>()

    for (const tag of tags) {
      const tagMembers = await this.store.getSetMembers(buildTagKey(tag))
      for (const key of tagMembers) {
        keys.add(key)
      }
    }

    await this.store.del([...keys, ...tags.map(buildTagKey)])
  }

  async waitForIdle(): Promise<void> {
    await Promise.allSettled([
      ...this.inflight.values(),
      ...this.backgroundRefreshes.values(),
    ])
  }

  private async load<T>(
    cacheKey: string,
    options: GithubCacheGetOrLoadOptions<T>,
  ): Promise<GithubCacheLoadResult<T>> {
    const entry = await this.readEntry<T>(cacheKey)
    const currentTime = this.now()

    if (entry && entry.freshUntil > currentTime) {
      return {
        payload: entry.payload,
        cacheStatus: 'hit',
      }
    }

    if (entry && entry.staleUntil > currentTime) {
      this.scheduleRefresh(cacheKey, options)
      return {
        payload: entry.payload,
        cacheStatus: 'stale',
      }
    }

    return this.loadAndStore(cacheKey, options)
  }

  private scheduleRefresh<T>(cacheKey: string, options: GithubCacheGetOrLoadOptions<T>) {
    if (this.backgroundRefreshes.has(cacheKey)) {
      return
    }

    const task = (async () => {
      const lockKey = buildLockKey(cacheKey)
      const lockValue = `${cacheKey}:${this.now()}`
      const lockTtlMs = options.lockTtlMs ?? 5_000
      const acquiredLock = await this.store.setIfNotExists(lockKey, lockValue, lockTtlMs)

      if (!acquiredLock) {
        return
      }

      try {
        const payload = await options.load()
        await this.writeEntry(cacheKey, payload, options.ttlMs, options.staleMs, options.tags ?? [])
      }
      catch (error) {
        logger.warn({ error, cacheKey }, 'Failed to refresh stale GitHub cache entry')
      }
      finally {
        await this.store.releaseLock(lockKey, lockValue)
      }
    })()

    this.backgroundRefreshes.set(cacheKey, task)
    void task.finally(() => {
      this.backgroundRefreshes.delete(cacheKey)
    })
  }

  private async loadAndStore<T>(
    cacheKey: string,
    options: GithubCacheGetOrLoadOptions<T>,
  ): Promise<GithubCacheLoadResult<T>> {
    const lockKey = buildLockKey(cacheKey)
    const lockValue = `${cacheKey}:${this.now()}`
    const lockTtlMs = options.lockTtlMs ?? 5_000
    const acquiredLock = await this.store.setIfNotExists(lockKey, lockValue, lockTtlMs)

    if (!acquiredLock) {
      const waitedEntry = await this.waitForEntry<T>(
        cacheKey,
        options.waitForRefreshMs ?? Math.min(lockTtlMs, 1_000),
      )

      if (waitedEntry) {
        return waitedEntry
      }
    }

    try {
      const payload = await options.load()
      await this.writeEntry(cacheKey, payload, options.ttlMs, options.staleMs, options.tags ?? [])

      return {
        payload,
        cacheStatus: 'miss',
      }
    }
    catch (error) {
      const staleEntry = await this.readEntry<T>(cacheKey)
      if (staleEntry && staleEntry.staleUntil > this.now()) {
        logger.warn({ error, cacheKey }, 'Serving stale GitHub cache entry after load failure')
        return {
          payload: staleEntry.payload,
          cacheStatus: 'stale',
        }
      }

      throw error
    }
    finally {
      if (acquiredLock) {
        await this.store.releaseLock(lockKey, lockValue)
      }
    }
  }

  private async waitForEntry<T>(
    cacheKey: string,
    waitForRefreshMs: number,
  ): Promise<GithubCacheLoadResult<T> | null> {
    const deadline = this.now() + waitForRefreshMs

    while (this.now() < deadline) {
      const entry = await this.readEntry<T>(cacheKey)
      const currentTime = this.now()

      if (entry && entry.freshUntil > currentTime) {
        return {
          payload: entry.payload,
          cacheStatus: 'hit',
        }
      }

      if (entry && entry.staleUntil > currentTime) {
        return {
          payload: entry.payload,
          cacheStatus: 'stale',
        }
      }

      await sleep(50)
    }

    return null
  }

  private async readEntry<T>(cacheKey: string): Promise<GithubCacheEntry<T> | null> {
    const rawValue = await this.store.get(cacheKey)
    const entry = parseCacheEntry<T>(rawValue)

    if (!rawValue || entry) {
      return entry
    }

    await this.store.del([cacheKey])
    return null
  }

  private async writeEntry<T>(
    cacheKey: string,
    payload: T,
    ttlMs: number,
    staleMs: number,
    tags: string[],
  ) {
    const previousEntry = await this.readEntry<unknown>(cacheKey)
    const uniqueTags = [...new Set(tags)].sort()
    const fetchedAt = this.now()
    const freshUntil = fetchedAt + ttlMs

    const nextEntry: GithubCacheEntry<T> = {
      payload,
      tags: uniqueTags,
      fetchedAt,
      freshUntil,
      staleUntil: freshUntil + staleMs,
    }

    await this.store.set(cacheKey, JSON.stringify(nextEntry))

    if (previousEntry?.tags.length) {
      await Promise.all(previousEntry.tags.map(tag => this.store.removeFromSet(buildTagKey(tag), [cacheKey])))
    }

    if (uniqueTags.length) {
      await Promise.all(uniqueTags.map(tag => this.store.addToSet(buildTagKey(tag), [cacheKey])))
    }
  }

  private logCacheResolution<T>(
    options: GithubCacheGetOrLoadOptions<T>,
    cacheKey: string,
    cacheStatus: GithubCacheStatus,
  ) {
    logger.info({
      cacheKey,
      cacheStatus,
      resourceKey: options.resourceKey,
      scope: options.scope,
      scopeId: options.scopeId ?? null,
    }, 'GitHub cache resolved')
  }
}
