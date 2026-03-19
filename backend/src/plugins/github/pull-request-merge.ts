import type {
  GithubPullRequestMergeMethod,
  GithubPullRequestMergeReadiness,
  GithubRepositoryResponse,
  PullRequestDetailsResponse,
  PullRequestParams,
} from './types.js'
import { logger } from '../../lib/logger.js'
import { fetchGithubPullRequest, fetchGithubRepository } from './service.js'

const MERGEABILITY_RETRY_ATTEMPTS = 3
const MERGEABILITY_RETRY_DELAY_MS = 150

function sleep(ms: number) {
  return new Promise(resolve => setTimeout(resolve, ms))
}

function resolveRepositoryMergeMetadata(
  pullRequest: PullRequestDetailsResponse,
  repository?: GithubRepositoryResponse | null,
) {
  const pullRequestRepo = pullRequest.base.repo

  return {
    permissions: pullRequestRepo.permissions ?? repository?.permissions ?? null,
    allow_merge_commit: pullRequestRepo.allow_merge_commit ?? repository?.allow_merge_commit ?? false,
    allow_squash_merge: pullRequestRepo.allow_squash_merge ?? repository?.allow_squash_merge ?? false,
    allow_rebase_merge: pullRequestRepo.allow_rebase_merge ?? repository?.allow_rebase_merge ?? false,
    source: repository ? 'repository' : 'pull_request',
  }
}

function shouldFetchRepositoryMergeMetadata(pullRequest: PullRequestDetailsResponse) {
  const pullRequestRepo = pullRequest.base.repo

  return pullRequestRepo.permissions == null
    || pullRequestRepo.allow_merge_commit == null
    || pullRequestRepo.allow_squash_merge == null
    || pullRequestRepo.allow_rebase_merge == null
}

function viewerCanMerge(pullRequest: PullRequestDetailsResponse, repository?: GithubRepositoryResponse | null) {
  const permissions = resolveRepositoryMergeMetadata(pullRequest, repository).permissions
  return Boolean(permissions?.admin || permissions?.maintain || permissions?.push)
}

function availableMergeMethods(
  pullRequest: PullRequestDetailsResponse,
  repository?: GithubRepositoryResponse | null,
): GithubPullRequestMergeMethod[] {
  const repositoryMetadata = resolveRepositoryMergeMetadata(pullRequest, repository)
  const methods: GithubPullRequestMergeMethod[] = []

  if (repositoryMetadata.allow_merge_commit) {
    methods.push('merge')
  }

  if (repositoryMetadata.allow_squash_merge) {
    methods.push('squash')
  }

  if (repositoryMetadata.allow_rebase_merge && pullRequest.rebaseable !== false) {
    methods.push('rebase')
  }

  return methods
}

function defaultMergeMethod(methods: GithubPullRequestMergeMethod[]) {
  return methods.at(0) ?? null
}

function blockedMergeMessage(mergeableState: string | null | undefined) {
  switch ((mergeableState ?? '').trim().toLowerCase()) {
    case 'dirty':
      return 'This pull request has merge conflicts that must be resolved before it can be merged.'
    case 'behind':
      return 'This pull request branch is out of date with the base branch.'
    case 'blocked':
      return 'This pull request is blocked by required reviews, checks, or repository rules.'
    case 'unstable':
      return 'GitHub is still finalizing merge checks for this pull request.'
    default:
      return 'GitHub reports that this pull request cannot be merged right now.'
  }
}

function isReadyMergeableState(mergeableState: string | null | undefined) {
  const normalizedState = (mergeableState ?? '').trim().toLowerCase()
  return normalizedState === '' || normalizedState === 'clean' || normalizedState === 'has_hooks'
}

export function buildGithubPullRequestMergeReadiness(
  pullRequest: PullRequestDetailsResponse,
  repository?: GithubRepositoryResponse | null,
): GithubPullRequestMergeReadiness {
  const canViewerMerge = viewerCanMerge(pullRequest, repository)
  const methods = availableMergeMethods(pullRequest, repository)
  const mergeableState = pullRequest.mergeable_state ?? null
  const readinessBase = {
    current_head_sha: pullRequest.head.sha,
    available_methods: methods,
    default_method: defaultMergeMethod(methods),
    viewer_can_merge: canViewerMerge,
    mergeable_state: mergeableState,
    rebaseable: pullRequest.rebaseable ?? null,
    auto_merge_enabled: Boolean(pullRequest.auto_merge),
  } satisfies Omit<
    GithubPullRequestMergeReadiness,
    'status' | 'message' | 'can_merge_now'
  >

  if (pullRequest.merged || pullRequest.merged_at) {
    return {
      ...readinessBase,
      status: 'merged',
      message: 'This pull request has already been merged.',
      can_merge_now: false,
    }
  }

  if (pullRequest.draft) {
    return {
      ...readinessBase,
      status: 'draft',
      message: 'This pull request is still marked as a draft.',
      can_merge_now: false,
    }
  }

  if (pullRequest.state !== 'open') {
    return {
      ...readinessBase,
      status: 'closed',
      message: 'This pull request is closed.',
      can_merge_now: false,
    }
  }

  if (!canViewerMerge) {
    return {
      ...readinessBase,
      status: 'forbidden',
      message: 'You do not have permission to merge this pull request.',
      can_merge_now: false,
    }
  }

  if (methods.length === 0) {
    return {
      ...readinessBase,
      status: 'blocked',
      message: 'Merging is disabled for this repository.',
      can_merge_now: false,
    }
  }

  if (pullRequest.mergeable == null) {
    return {
      ...readinessBase,
      status: 'checking',
      message: 'GitHub is still computing whether this pull request can be merged.',
      can_merge_now: false,
    }
  }

  if (pullRequest.mergeable && isReadyMergeableState(mergeableState)) {
    return {
      ...readinessBase,
      status: 'ready',
      message: 'This pull request is ready to merge.',
      can_merge_now: true,
    }
  }

  return {
    ...readinessBase,
    status: 'blocked',
    message: blockedMergeMessage(mergeableState),
    can_merge_now: false,
  }
}

export async function fetchGithubPullRequestMergeReadiness(
  {
    token,
    params,
    fetchPullRequest = fetchGithubPullRequest,
    fetchRepository = fetchGithubRepository,
  }: {
    token: string
    params: PullRequestParams
    fetchPullRequest?: (input: { token: string, params: PullRequestParams }) => Promise<PullRequestDetailsResponse>
    fetchRepository?: (input: {
      token: string
      params: { owner: string, repo: string }
    }) => Promise<GithubRepositoryResponse>
  },
): Promise<GithubPullRequestMergeReadiness> {
  let lastReadiness: GithubPullRequestMergeReadiness | null = null
  let repositoryMetadata: GithubRepositoryResponse | null = null

  for (let attempt = 0; attempt < MERGEABILITY_RETRY_ATTEMPTS; attempt += 1) {
    const pullRequest = await fetchPullRequest({ token, params })

    if (!repositoryMetadata && shouldFetchRepositoryMergeMetadata(pullRequest)) {
      repositoryMetadata = await fetchRepository({
        token,
        params: {
          owner: params.owner,
          repo: params.repo,
        },
      })
    }

    const effectiveRepositoryMetadata = resolveRepositoryMergeMetadata(
      pullRequest,
      repositoryMetadata,
    )
    lastReadiness = buildGithubPullRequestMergeReadiness(pullRequest, repositoryMetadata)

    if (lastReadiness.status === 'forbidden') {
      logger.warn({
        owner: params.owner,
        repo: params.repo,
        pullNumber: params.pull_number,
        attempt: attempt + 1,
        pullRequestState: pullRequest.state,
        draft: pullRequest.draft,
        merged: pullRequest.merged,
        mergedAt: pullRequest.merged_at,
        mergeable: pullRequest.mergeable,
        mergeableState: pullRequest.mergeable_state ?? null,
        rebaseable: pullRequest.rebaseable ?? null,
        repoMetadataSource: effectiveRepositoryMetadata.source,
        repoPermissions: effectiveRepositoryMetadata.permissions,
        repoMergeSettings: {
          allowMergeCommit: effectiveRepositoryMetadata.allow_merge_commit,
          allowSquashMerge: effectiveRepositoryMetadata.allow_squash_merge,
          allowRebaseMerge: effectiveRepositoryMetadata.allow_rebase_merge,
        },
        availableMethods: lastReadiness.available_methods,
        viewerCanMerge: lastReadiness.viewer_can_merge,
      }, 'GitHub merge readiness resolved to forbidden')
    }

    if (pullRequest.mergeable != null || lastReadiness.status !== 'checking') {
      return lastReadiness
    }

    if (attempt < MERGEABILITY_RETRY_ATTEMPTS - 1) {
      logger.info({
        owner: params.owner,
        repo: params.repo,
        pullNumber: params.pull_number,
        attempt: attempt + 1,
        mergeable: pullRequest.mergeable,
        mergeableState: pullRequest.mergeable_state ?? null,
      }, 'GitHub mergeability is still pending; retrying merge readiness')
      await sleep(MERGEABILITY_RETRY_DELAY_MS)
    }
  }

  return lastReadiness ?? {
    status: 'checking',
    message: 'GitHub is still computing whether this pull request can be merged.',
    current_head_sha: '',
    available_methods: [],
    default_method: null,
    can_merge_now: false,
    viewer_can_merge: false,
    mergeable_state: null,
    rebaseable: null,
    auto_merge_enabled: false,
  }
}
