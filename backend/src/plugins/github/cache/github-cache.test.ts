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
      return {
        payload: [`value-${loadCount}`],
      }
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
      scope: 'viewer',
    })
    expect(second).toEqual({
      payload: ['value-1'],
      cacheStatus: 'hit',
      scope: 'viewer',
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
      return {
        payload: [`value-${loadCount}`],
      }
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
      scope: 'viewer',
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
      scope: 'viewer',
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
      load: async () => ({
        payload: ['value-1'],
      }),
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
      scope: 'viewer',
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
      return {
        payload: [`value-${loadCount}`],
      }
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
      scope: 'viewer',
    })
    expect(loadCount).toBe(2)
  })

  it('serves a primed public entry without reloading GitHub', async () => {
    const cache = createGithubCache({
      store: new MemoryGithubCacheStore(),
    })

    await cache.prime({
      scope: 'public',
      resourceKey: 'repo:openai/reviu:details',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['repo:openai/reviu'],
      payload: {
        full_name: 'OpenAI/Reviu',
      },
    })

    const load = vi.fn(async () => ({
      payload: {
        full_name: 'should-not-load',
      },
    }))

    const result = await cache.getOrLoad({
      scope: 'public',
      resourceKey: 'repo:openai/reviu:details',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['repo:openai/reviu'],
      load,
    })

    expect(result).toEqual({
      payload: {
        full_name: 'OpenAI/Reviu',
      },
      cacheStatus: 'hit',
      scope: 'public',
    })
    expect(load).not.toHaveBeenCalled()
  })

  it('revalidates an expired entry when GitHub returns not modified', async () => {
    let currentTime = 1_000
    let loadCount = 0
    const cache = createGithubCache({
      store: new MemoryGithubCacheStore(),
      now: () => currentTime,
    })

    const load = async ({ cachedEntry }: { cachedEntry: { etag?: string } | null }) => {
      loadCount += 1

      if (cachedEntry) {
        expect(cachedEntry.etag).toBe('"pull-request-v1"')
        return {
          notModified: true as const,
          etag: '"pull-request-v1"',
        }
      }

      return {
        payload: ['value-1'],
        etag: '"pull-request-v1"',
      }
    }

    await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:42',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    currentTime += 360_001

    const revalidated = await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'pull-request:42',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    expect(revalidated).toEqual({
      payload: ['value-1'],
      cacheStatus: 'hit',
      scope: 'viewer',
    })
    expect(loadCount).toBe(2)
  })

  it('preserves named validators across not-modified revalidation', async () => {
    let currentTime = 1_000
    let loadCount = 0
    const cache = createGithubCache({
      store: new MemoryGithubCacheStore(),
      now: () => currentTime,
    })

    const load = async ({
      cachedEntry,
    }: {
      cachedEntry: {
        validators?: Record<string, { etag?: string }>
      } | null
    }) => {
      loadCount += 1

      if (loadCount === 1) {
        return {
          payload: ['value-1'],
          etag: '"issue-v1"',
          validators: {
            issue: {
              etag: '"issue-v1"',
            },
            issueComments: {
              etag: '"comments-v1"',
            },
          },
        }
      }

      expect(cachedEntry?.validators).toEqual({
        issue: {
          etag: '"issue-v1"',
        },
        issueComments: {
          etag: '"comments-v1"',
        },
      })

      return {
        notModified: true as const,
        validators: {
          issue: {
            etag: '"issue-v1"',
          },
          issueComments: {
            etag: '"comments-v1"',
          },
        },
      }
    }

    await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'issue:42',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    currentTime += 360_001

    const revalidated = await cache.getOrLoad({
      scope: 'viewer',
      scopeId: 'user-1',
      resourceKey: 'issue:42',
      ttlMs: 60_000,
      staleMs: 300_000,
      tags: ['viewer:user-1'],
      load,
    })

    expect(revalidated).toEqual({
      payload: ['value-1'],
      cacheStatus: 'hit',
      scope: 'viewer',
    })
    expect(loadCount).toBe(2)
  })
})
