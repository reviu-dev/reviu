import { createGithubCache } from './github-cache.js'
import { createDefaultGithubCacheStore } from './redis.js'

export const githubCache = createGithubCache({
  store: createDefaultGithubCacheStore(),
})
