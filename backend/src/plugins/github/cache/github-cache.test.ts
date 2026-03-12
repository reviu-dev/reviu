import { afterEach, describe, expect, it, vi } from 'vitest'

import { logger } from '../../../lib/logger.js'
import { createGithubCache, MemoryGithubCacheStore } from './github-cache.js'

describe('github cache', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('returns a fresh hit after the initial miss', async () => {
    let currentTime = 1_000
    let loadCount = 0
    const loggerInfoSpy = vi.spyOn(logger, 'info').mockImplementation(() => undefined)
    const cache = createGithubCache({
      store: new MemoryGithubCacheStore(),
      now: () => currentTime,
    })

    const load = async () => {
      loadCount += 1
      return [`value-${loadCount}`]
    }

    const first = await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    const second = await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    expect(first).toEqual({
      payload: ['value-1'],
      cacheStatus: 'miss',
    })
    expect(second).toEqual({
      payload: ['value-1'],
      cacheStatus: 'hit',
    })
    expect(loadCount).toBe(1)
    expect(loggerInfoSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        cacheStatus: 'hit',
        resourceKey: 'search:latest',
        scope: 'viewer',
        scopeId: 'user-1',
      }),
      'GitHub cache resolved',
    )

    currentTime += 1
  })

  it('serves stale data and refreshes it in the background', async () => {
    let currentTime = 1_000
    let loadCount = 0
    const cache = createGithubCache({
      store: new MemoryGithubCacheStore(),
      now: () => currentTime,
    })

    const load = async () => {
      loadCount += 1
      return [`value-${loadCount}`]
    }

    await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    currentTime += 60_001

    const stale = await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    expect(stale).toEqual({
      payload: ['value-1'],
      cacheStatus: 'stale',
    })

    await cache.waitForIdle()

    const refreshed = await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    expect(refreshed).toEqual({
      payload: ['value-2'],
      cacheStatus: 'hit',
    })
    expect(loadCount).toBe(2)
  })

  it('serves stale data when refresh fails', async () => {
    let currentTime = 1_000
    const cache = createGithubCache({
      store: new MemoryGithubCacheStore(),
      now: () => currentTime,
    })

    await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load: async () => ['value-1'],
    })

    currentTime += 60_001

    const stale = await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load: async () => {
        throw new Error('upstream failed')
      },
    })

    expect(stale).toEqual({
      payload: ['value-1'],
      cacheStatus: 'stale',
    })

    await cache.waitForIdle()
  })

  it('invalidates cache entries by tag', async () => {
    let loadCount = 0
    const cache = createGithubCache({
      store: new MemoryGithubCacheStore(),
    })

    const load = async () => {
      loadCount += 1
      return [`value-${loadCount}`]
    }

    await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    await cache.invalidateTags(['viewer:user-1'])

    const reloaded = await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'search:latest',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    expect(reloaded).toEqual({
      payload: ['value-2'],
      cacheStatus: 'miss',
    })
    expect(loadCount).toBe(2)
  })
})
