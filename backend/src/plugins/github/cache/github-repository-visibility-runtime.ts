import { createDefaultGithubCacheStore } from '../../../lib/redis.js'
import { createGithubRepositoryVisibilityStore } from './github-repository-visibility.js'

export const githubRepositoryVisibility = createGithubRepositoryVisibilityStore({
  store: createDefaultGithubCacheStore(),
})
