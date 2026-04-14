import type {
  GithubCacheGetOrLoadOptions,
  GithubCacheLoadResult,
  GithubCachePrimeOptions,
  GithubCacheStore,
} from './github-cache.js'

import { env } from '../../../lib/env.js'
import { logger } from '../../../lib/logger.js'
import { createDefaultGithubCacheStore } from '../../../lib/redis.js'
import { createGithubCache } from './github-cache.js'

type GithubCacheRuntime = Pick<
  ReturnType<typeof createGithubCache>,
  'getOrLoad' | 'invalidateTags' | 'prime' | 'waitForIdle'
>

function createGithubCacheRuntime(
  {
    cacheEnabled,
    store,
  }: {
    cacheEnabled: boolean
    store: GithubCacheStore
  },
): GithubCacheRuntime {
  if (cacheEnabled) {
    return createGithubCache({ store })
  }

  logger.warn('GitHub cache is disabled by env')

  return {
    async getOrLoad<T>(options: GithubCacheGetOrLoadOptions<T>): Promise<GithubCacheLoadResult<T>> {
      const result = await options.load({ cachedEntry: null })

      if ('notModified' in result) {
        throw new Error(`GitHub cache bypass cannot revalidate ${options.resourceKey} without a cached entry`)
      }

      return {
        payload: result.payload,
        cacheStatus: 'miss',
        scope: options.scope,
      }
    },
    async invalidateTags(): Promise<void> {},
    async prime<T>(_options: GithubCachePrimeOptions<T>): Promise<void> {},
    async waitForIdle(): Promise<void> {},
  }
}

export const githubCache = createGithubCacheRuntime({
  cacheEnabled: env.GITHUB_CACHE_ENABLED,
  store: createDefaultGithubCacheStore(),
})
