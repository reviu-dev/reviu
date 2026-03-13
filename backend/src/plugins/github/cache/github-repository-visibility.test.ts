import { describe, expect, it } from 'vitest'
import { MemoryGithubCacheStore } from './github-cache.js'
import { createGithubRepositoryVisibilityStore } from './github-repository-visibility.js'

describe('github repository visibility store', () => {
  it('marks repositories as publicly shareable for a bounded time window', async () => {
    let currentTime = 1_000
    const visibilityStore = createGithubRepositoryVisibilityStore({
      store: new MemoryGithubCacheStore(),
      now: () => currentTime,
      ttlMs: 60_000,
    })

    expect(await visibilityStore.isKnownPublic('OpenAI', 'Reviu')).toBe(false)

    await visibilityStore.markPublic('OpenAI', 'Reviu')

    expect(await visibilityStore.isKnownPublic('openai', 'reviu')).toBe(true)

    currentTime += 60_001

    expect(await visibilityStore.isKnownPublic('OpenAI', 'Reviu')).toBe(false)
  })

  it('clears the public marker explicitly', async () => {
    const visibilityStore = createGithubRepositoryVisibilityStore({
      store: new MemoryGithubCacheStore(),
    })

    await visibilityStore.markPublic('OpenAI', 'Reviu')
    await visibilityStore.clear('OpenAI', 'Reviu')

    expect(await visibilityStore.isKnownPublic('OpenAI', 'Reviu')).toBe(false)
  })
})
