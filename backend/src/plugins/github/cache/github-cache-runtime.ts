import { createDefaultGithubCacheStore } from '../../../lib/redis.js'
import { createGithubCache } from './github-cache.js'

export const githubCache = createGithubCache({
  store: createDefaultGithubCacheStore(),
})
