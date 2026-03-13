import { logger } from '../../../lib/logger.js'
import { getGithubMetricsContext } from '../metrics/github-metrics-context.js'
import { githubMetricsCollector } from '../metrics/github-metrics.js'

export type GithubCacheScope = 'viewer' | 'installation' | 'public'
export type GithubCacheStatus = 'hit' | 'miss' | 'stale'

export interface GithubCacheEntry<T> {
  payload: T
  tags: string[]
  fetchedAt: number
  freshUntil: number
  staleUntil: number
  etag?: string
  lastModified?: string
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
  operation?: string
  scope: GithubCacheScope
  scopeId?: string
  resourceKey: string
  ttlMs: number
  staleMs: number
  lockTtlMs?: number
  waitForRefreshMs?: number
  tags?: string[]
  load: (context: GithubCacheLoadContext<T>) => Promise<GithubCacheLoaderResult<T>>
}

export interface GithubCacheLoadResult<T> {
  payload: T
  cacheStatus: GithubCacheStatus
}

export interface GithubCachePrimeOptions<T> {
  scope: GithubCacheScope
  scopeId?: string
  resourceKey: string
  ttlMs: number
  staleMs: number
  tags?: string[]
  payload: T
  etag?: string
  lastModified?: string
}

export interface GithubCacheLoadContext<T> {
  cachedEntry: GithubCacheEntry<T> | null
}

export interface GithubCacheLoadedPayload<T> {
  payload: T
  etag?: string
  lastModified?: string
}

export interface GithubCacheNotModifiedPayload {
  notModified: true
  etag?: string
  lastModified?: string
}

export type GithubCacheLoaderResult<T> = GithubCacheLoadedPayload<T> | GithubCacheNotModifiedPayload

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

function isNotModifiedPayload<T>(
  value: GithubCacheLoaderResult<T>,
): value is GithubCacheNotModifiedPayload {
  return 'notModified' in value && value.notModified === true
}

class GithubCacheManager {
  private readonly inflight = new Map<string, Promise<GithubCacheLoadResult<unknown>>>()
  private readonly backgroundRefreshes = new Map<string, Promise<void>>()

  constructor(
    private readonly store: GithubCacheStore,
    private readonly now: () => number,
  ) {}

  async getOrLoad<T>(options: GithubCacheGetOrLoadOptions<T>): Promise<GithubCacheLoadResult<T>> {
    const startedAt = this.now()
    const cacheKey = buildCacheKey(options.scope, options.resourceKey, options.scopeId)
    const inflight = this.inflight.get(cacheKey) as Promise<GithubCacheLoadResult<T>> | undefined
    if (inflight) {
      const result = await inflight
      this.logCacheResolution(options, cacheKey, result.cacheStatus)
      this.recordCacheResolution(options, result.cacheStatus, startedAt)
      return result
    }

    const task = this.load(cacheKey, options)
    this.inflight.set(cacheKey, task as Promise<GithubCacheLoadResult<unknown>>)

    try {
      const result = await task
      this.logCacheResolution(options, cacheKey, result.cacheStatus)
      this.recordCacheResolution(options, result.cacheStatus, startedAt)
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

  async prime<T>(options: GithubCachePrimeOptions<T>): Promise<void> {
    const cacheKey = buildCacheKey(options.scope, options.resourceKey, options.scopeId)

    await this.writeEntry(
      cacheKey,
      options.payload,
      options.ttlMs,
      options.staleMs,
      options.tags ?? [],
      {
        etag: options.etag,
        lastModified: options.lastModified,
      },
    )
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

    return this.loadAndStore(cacheKey, entry, options)
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
        const cachedEntry = await this.readEntry<T>(cacheKey)
        if (!cachedEntry) {
          return
        }

        const loadResult = await options.load({ cachedEntry })
        if (isNotModifiedPayload(loadResult)) {
          await this.writeEntry(
            cacheKey,
            cachedEntry.payload,
            options.ttlMs,
            options.staleMs,
            options.tags ?? [],
            {
              etag: loadResult.etag ?? cachedEntry.etag,
              lastModified: loadResult.lastModified ?? cachedEntry.lastModified,
            },
          )
          return
        }

        await this.writeEntry(
          cacheKey,
          loadResult.payload,
          options.ttlMs,
          options.staleMs,
          options.tags ?? [],
          {
            etag: loadResult.etag,
            lastModified: loadResult.lastModified,
          },
        )
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
    cachedEntry: GithubCacheEntry<T> | null,
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
      const currentEntry = await this.readEntry<T>(cacheKey)
      const loadResult = await options.load({ cachedEntry: currentEntry })

      if (isNotModifiedPayload(loadResult)) {
        if (!currentEntry) {
          throw new Error(`GitHub cache loader returned notModified without an existing entry for ${cacheKey}`)
        }

        await this.writeEntry(
          cacheKey,
          currentEntry.payload,
          options.ttlMs,
          options.staleMs,
          options.tags ?? [],
          {
            etag: loadResult.etag ?? currentEntry.etag,
            lastModified: loadResult.lastModified ?? currentEntry.lastModified,
          },
        )

        return {
          payload: currentEntry.payload,
          cacheStatus: 'hit',
        }
      }

      await this.writeEntry(
        cacheKey,
        loadResult.payload,
        options.ttlMs,
        options.staleMs,
        options.tags ?? [],
        {
          etag: loadResult.etag,
          lastModified: loadResult.lastModified,
        },
      )

      return {
        payload: loadResult.payload,
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
    metadata?: {
      etag?: string
      lastModified?: string
    },
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
      etag: metadata?.etag,
      lastModified: metadata?.lastModified,
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

  private recordCacheResolution<T>(
    options: GithubCacheGetOrLoadOptions<T>,
    cacheStatus: GithubCacheStatus,
    startedAt: number,
  ) {
    const context = getGithubMetricsContext()
    const operation = context?.operation ?? options.operation

    if (!operation) {
      return
    }

    githubMetricsCollector.recordCacheEvent({
      userId: context?.userId,
      operation,
      scope: options.scope,
      cacheStatus,
      ttlMs: options.ttlMs,
      staleMs: options.staleMs,
      durationMs: Math.max(this.now() - startedAt, 0),
    })
  }
}
