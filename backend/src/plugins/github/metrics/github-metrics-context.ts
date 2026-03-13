import { AsyncLocalStorage } from 'node:async_hooks'

export interface GithubMetricsContext {
  userId?: string
  operation?: string
}

const githubMetricsContextStorage = new AsyncLocalStorage<GithubMetricsContext>()

export function runWithGithubMetricsContext<T>(
  context: GithubMetricsContext,
  callback: () => T,
) {
  return githubMetricsContextStorage.run(context, callback)
}

export function getGithubMetricsContext() {
  return githubMetricsContextStorage.getStore() ?? null
}
