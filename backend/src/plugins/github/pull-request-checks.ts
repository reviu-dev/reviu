import type {
  BranchRulesResponse,
  CommitCheckRunsResponse,
  CommitCombinedStatusResponse,
  GithubPullRequestCheckRun,
  GithubPullRequestChecksRollupState,
  GithubPullRequestChecksSummary,
  GithubPullRequestLegacyStatus,
  GithubPullRequestWorkflowJob,
  GithubPullRequestWorkflowRun,
  GithubPullRequestWorkflowStep,
  PullRequestDetailsResponse,
  PullRequestParams,
  WorkflowRunJobsResponse,
  WorkflowRunsResponse,
} from './types.js'
import {
  fetchGithubBranchRules,
  fetchGithubCombinedStatusForRef,
  fetchGithubCommitCheckRuns,
  fetchGithubPullRequest,
  fetchGithubWorkflowRunJobs,
  fetchGithubWorkflowRuns,
} from './service.js'

const PENDING_CHECK_STATUSES = new Set([
  'queued',
  'in_progress',
  'pending',
  'requested',
  'waiting',
])

const SUCCESS_CHECK_CONCLUSIONS = new Set([
  'success',
  'neutral',
  'skipped',
])

const FAILURE_CHECK_CONCLUSIONS = new Set([
  'action_required',
  'cancelled',
  'error',
  'failure',
  'stale',
  'timed_out',
])

interface GithubRequiredStatusCheckRule {
  type?: string | null
  parameters?: {
    required_status_checks?: Array<{
      context?: string | null
    }> | null
    strict_required_status_checks_policy?: boolean | null
  } | null
}

interface CountableCheckItem {
  state: GithubPullRequestChecksRollupState
  required: boolean
}

interface CheckRunAppMetadata {
  name: string | null
  slug: string | null
  avatarUrl: string | null
}

function normalizeCheckState(
  status: string | null | undefined,
  conclusion: string | null | undefined,
): GithubPullRequestChecksRollupState {
  const normalizedStatus = status?.trim().toLowerCase() ?? ''
  if (PENDING_CHECK_STATUSES.has(normalizedStatus) || normalizedStatus === '') {
    return 'pending'
  }

  const normalizedConclusion = conclusion?.trim().toLowerCase() ?? ''
  if (SUCCESS_CHECK_CONCLUSIONS.has(normalizedConclusion)) {
    return 'success'
  }

  if (FAILURE_CHECK_CONCLUSIONS.has(normalizedConclusion)) {
    return 'failure'
  }

  return normalizedStatus === 'completed' ? 'pending' : 'pending'
}

function rollupStates(states: GithubPullRequestChecksRollupState[]): GithubPullRequestChecksRollupState {
  if (states.includes('failure')) {
    return 'failure'
  }

  if (states.includes('pending')) {
    return 'pending'
  }

  return 'success'
}

function summarizeCountableChecks(items: CountableCheckItem[]) {
  const successfulChecks = items.filter(item => item.state === 'success').length
  const failedChecks = items.filter(item => item.state === 'failure').length
  const pendingChecks = items.filter(item => item.state === 'pending').length

  return {
    total_checks: items.length,
    successful_checks: successfulChecks,
    failed_checks: failedChecks,
    pending_checks: pendingChecks,
    overall_state: rollupStates(items.map(item => item.state)),
  }
}

function sortStrings(values: Iterable<string>) {
  return [...values].sort((left, right) => left.localeCompare(right))
}

function extractRequiredStatusCheckConfig(
  branchRules: BranchRulesResponse | null | undefined,
) {
  const requiredContexts = new Set<string>()
  let requiresUpToDateBranch = false

  for (const rule of (branchRules ?? []) as GithubRequiredStatusCheckRule[]) {
    if (rule.type !== 'required_status_checks') {
      continue
    }

    if (rule.parameters?.strict_required_status_checks_policy) {
      requiresUpToDateBranch = true
    }

    for (const check of rule.parameters?.required_status_checks ?? []) {
      const context = check.context?.trim()
      if (context) {
        requiredContexts.add(context)
      }
    }
  }

  return {
    requiredContexts: sortStrings(requiredContexts),
    requiresUpToDateBranch,
  }
}

function parseCheckRunIdFromUrl(url: string | null | undefined) {
  if (!url) {
    return null
  }

  const match = url.match(/\/check-runs\/(\d+)(?:\/|$)/)
  if (!match) {
    return null
  }

  const checkRunId = Number.parseInt(match[1], 10)
  return Number.isNaN(checkRunId) ? null : checkRunId
}

function legacyStatusAvatarUrl(status: CommitCombinedStatusResponse['statuses'][number]) {
  return (status as { avatar_url?: string | null }).avatar_url ?? null
}

function compareDescendingIsoTimestamps(
  left: string | null | undefined,
  right: string | null | undefined,
) {
  return (right ?? '').localeCompare(left ?? '')
}

function dedupeLegacyStatuses(
  combinedStatus: CommitCombinedStatusResponse | null | undefined,
) {
  const latestStatusesByContext = new Map<string, CommitCombinedStatusResponse['statuses'][number]>()

  for (const status of combinedStatus?.statuses ?? []) {
    const context = status.context?.trim()
    if (!context) {
      continue
    }

    const existing = latestStatusesByContext.get(context)
    if (!existing || compareDescendingIsoTimestamps(status.updated_at, existing.updated_at) < 0) {
      latestStatusesByContext.set(context, status)
    }
  }

  return [...latestStatusesByContext.values()].sort((left, right) =>
    compareDescendingIsoTimestamps(left.updated_at, right.updated_at)
    || left.context.localeCompare(right.context),
  )
}

export function buildGithubPullRequestChecksSummary({
  pullRequest,
  branchRules,
  checkRuns,
  workflowRuns,
  combinedStatus,
  workflowRunJobsByRunId = new Map<number, WorkflowRunJobsResponse>(),
}: {
  pullRequest: PullRequestDetailsResponse
  branchRules?: BranchRulesResponse | null
  checkRuns?: CommitCheckRunsResponse | null
  workflowRuns?: WorkflowRunsResponse | null
  combinedStatus?: CommitCombinedStatusResponse | null
  workflowRunJobsByRunId?: Map<number, WorkflowRunJobsResponse>
}): GithubPullRequestChecksSummary {
  const { requiredContexts, requiresUpToDateBranch } = extractRequiredStatusCheckConfig(branchRules)
  const requiredContextSet = new Set(requiredContexts)
  const checkRunAppsById = new Map<number, CheckRunAppMetadata>()

  for (const checkRun of checkRuns?.check_runs ?? []) {
    checkRunAppsById.set(checkRun.id, {
      name: checkRun.app?.name ?? null,
      slug: checkRun.app?.slug ?? null,
      avatarUrl: checkRun.app?.owner?.avatar_url ?? null,
    })
  }

  const actionsRuns = (workflowRuns?.workflow_runs ?? [])
    .filter(run => run.head_sha === pullRequest.head.sha)
    .sort((left, right) =>
      compareDescendingIsoTimestamps(left.created_at, right.created_at)
      || right.id - left.id,
    )
    .map((run): GithubPullRequestWorkflowRun => {
      const jobs = (workflowRunJobsByRunId.get(run.id)?.jobs ?? [])
        .map((job): GithubPullRequestWorkflowJob => {
          const checkRunId = parseCheckRunIdFromUrl(job.check_run_url)
          const app = checkRunId == null ? undefined : checkRunAppsById.get(checkRunId)
          const steps = (job.steps ?? [])
            .slice()
            .sort((left, right) => left.number - right.number)
            .map((step): GithubPullRequestWorkflowStep => ({
              number: step.number,
              name: step.name,
              status: step.status ?? null,
              conclusion: step.conclusion ?? null,
              state: normalizeCheckState(step.status, step.conclusion),
              started_at: step.started_at ?? null,
              completed_at: step.completed_at ?? null,
            }))

          return {
            id: job.id,
            name: job.name,
            status: job.status ?? null,
            conclusion: job.conclusion ?? null,
            state: normalizeCheckState(job.status, job.conclusion),
            started_at: job.started_at ?? null,
            completed_at: job.completed_at ?? null,
            html_url: job.html_url,
            required: requiredContextSet.has(job.name),
            app_name: app?.name ?? null,
            app_slug: app?.slug ?? null,
            app_avatar_url: app?.avatarUrl ?? null,
            steps,
          }
        })
        .sort((left, right) =>
          compareDescendingIsoTimestamps(left.started_at, right.started_at)
          || right.id - left.id,
        )

      const runState = jobs.length > 0
        ? rollupStates(jobs.map(job => job.state))
        : normalizeCheckState(run.status, run.conclusion)

      return {
        id: run.id,
        name: run.name ?? null,
        display_title: run.display_title ?? null,
        event: run.event,
        status: run.status ?? null,
        conclusion: run.conclusion ?? null,
        state: runState,
        created_at: run.created_at,
        updated_at: run.updated_at,
        run_started_at: run.run_started_at ?? null,
        run_number: run.run_number,
        run_attempt: run.run_attempt ?? null,
        html_url: run.html_url,
        jobs,
      }
    })

  const actionJobCheckRunIds = new Set<number>()
  for (const run of actionsRuns) {
    for (const job of workflowRunJobsByRunId.get(run.id)?.jobs ?? []) {
      const checkRunId = parseCheckRunIdFromUrl(job.check_run_url)
      if (checkRunId != null) {
        actionJobCheckRunIds.add(checkRunId)
      }
    }
  }

  const otherChecks = (checkRuns?.check_runs ?? [])
    .filter(checkRun => !actionJobCheckRunIds.has(checkRun.id))
    .sort((left, right) =>
      compareDescendingIsoTimestamps(left.started_at, right.started_at)
      || right.id - left.id,
    )
    .map((checkRun): GithubPullRequestCheckRun => ({
      id: checkRun.id,
      name: checkRun.name,
      status: checkRun.status ?? null,
      conclusion: checkRun.conclusion ?? null,
      state: normalizeCheckState(checkRun.status, checkRun.conclusion),
      started_at: checkRun.started_at ?? null,
      completed_at: checkRun.completed_at ?? null,
      html_url: checkRun.html_url,
      details_url: checkRun.details_url ?? null,
      required: requiredContextSet.has(checkRun.name),
      app_name: checkRun.app?.name ?? null,
      app_slug: checkRun.app?.slug ?? null,
      app_avatar_url: checkRun.app?.owner?.avatar_url ?? null,
      title: checkRun.output?.title ?? null,
      summary: checkRun.output?.summary ?? null,
      text: checkRun.output?.text ?? null,
      annotations_count: checkRun.output?.annotations_count ?? 0,
    }))

  const legacyStatuses = dedupeLegacyStatuses(combinedStatus)
    .map((status): GithubPullRequestLegacyStatus => ({
      id: status.id,
      context: status.context,
      status: status.state,
      state: normalizeCheckState(status.state, status.state),
      description: status.description ?? null,
      target_url: status.target_url ?? null,
      avatar_url: legacyStatusAvatarUrl(status),
      created_at: status.created_at,
      updated_at: status.updated_at,
      required: requiredContextSet.has(status.context),
    }))

  const observedRequiredContexts = new Set<string>()
  for (const run of actionsRuns) {
    for (const job of run.jobs) {
      if (job.required) {
        observedRequiredContexts.add(job.name)
      }
    }
  }

  for (const check of otherChecks) {
    if (check.required) {
      observedRequiredContexts.add(check.name)
    }
  }

  for (const status of legacyStatuses) {
    if (status.required) {
      observedRequiredContexts.add(status.context)
    }
  }

  const missingRequiredContexts = requiredContexts.filter(
    context => !observedRequiredContexts.has(context),
  )

  const countableChecks: CountableCheckItem[] = [
    ...actionsRuns.flatMap((run) => {
      if (run.jobs.length === 0) {
        return []
      }

      return run.jobs.map(job => ({
        state: job.state,
        required: job.required,
      }))
    }),
    ...actionsRuns
      .filter(run => run.jobs.length === 0)
      .map(run => ({
        state: run.state,
        required: Boolean(run.name && requiredContextSet.has(run.name)),
      })),
    ...otherChecks.map(check => ({
      state: check.state,
      required: check.required,
    })),
    ...legacyStatuses.map(status => ({
      state: status.state,
      required: status.required,
    })),
    ...missingRequiredContexts.map(() => ({
      state: 'pending' as const,
      required: true,
    })),
  ]

  const overallSummary = summarizeCountableChecks(countableChecks)
  const requiredSummary = summarizeCountableChecks(
    countableChecks.filter(item => item.required),
  )

  return {
    head_sha: pullRequest.head.sha,
    overall_state: overallSummary.overall_state,
    required_state: requiredSummary.overall_state,
    total_checks: overallSummary.total_checks,
    successful_checks: overallSummary.successful_checks,
    failed_checks: overallSummary.failed_checks,
    pending_checks: overallSummary.pending_checks,
    required_checks_total: requiredSummary.total_checks,
    required_checks_passed: requiredSummary.successful_checks,
    required_checks_failed: requiredSummary.failed_checks,
    required_checks_pending: requiredSummary.pending_checks,
    required_contexts: requiredContexts,
    missing_required_contexts: missingRequiredContexts,
    requires_up_to_date_branch: requiresUpToDateBranch,
    actions_runs: actionsRuns,
    other_checks: otherChecks,
    legacy_statuses: legacyStatuses,
  }
}

export async function fetchGithubPullRequestChecksSummary({
  token,
  params,
  fetchPullRequest = fetchGithubPullRequest,
  fetchBranchRules = fetchGithubBranchRules,
  fetchCommitCheckRuns = fetchGithubCommitCheckRuns,
  fetchCombinedStatus = fetchGithubCombinedStatusForRef,
  fetchWorkflowRuns = fetchGithubWorkflowRuns,
  fetchWorkflowRunJobs = fetchGithubWorkflowRunJobs,
}: {
  token: string
  params: PullRequestParams
  fetchPullRequest?: (input: { token: string, params: PullRequestParams }) => Promise<PullRequestDetailsResponse>
  fetchBranchRules?: (input: {
    token: string
    params: { owner: string, repo: string, branch: string }
  }) => Promise<BranchRulesResponse>
  fetchCommitCheckRuns?: (input: {
    token: string
    params: { owner: string, repo: string, ref: string, per_page?: number }
  }) => Promise<CommitCheckRunsResponse>
  fetchCombinedStatus?: (input: {
    token: string
    params: { owner: string, repo: string, ref: string }
  }) => Promise<CommitCombinedStatusResponse>
  fetchWorkflowRuns?: (input: {
    token: string
    params: { owner: string, repo: string, head_sha: string, per_page?: number }
  }) => Promise<WorkflowRunsResponse>
  fetchWorkflowRunJobs?: (input: {
    token: string
    params: { owner: string, repo: string, run_id: number }
  }) => Promise<WorkflowRunJobsResponse>
}): Promise<GithubPullRequestChecksSummary> {
  const pullRequest = await fetchPullRequest({ token, params })
  const owner = params.owner
  const repo = params.repo
  const headSha = pullRequest.head.sha

  const [branchRulesResult, checkRunsResult, combinedStatusResult, workflowRunsResult]
    = await Promise.allSettled([
      fetchBranchRules({
        token,
        params: {
          owner,
          repo,
          branch: pullRequest.base.ref,
        },
      }),
      fetchCommitCheckRuns({
        token,
        params: {
          owner,
          repo,
          ref: headSha,
          per_page: 100,
        },
      }),
      fetchCombinedStatus({
        token,
        params: {
          owner,
          repo,
          ref: headSha,
        },
      }),
      fetchWorkflowRuns({
        token,
        params: {
          owner,
          repo,
          head_sha: headSha,
          per_page: 100,
        },
      }),
    ])

  const workflowRuns
    = workflowRunsResult.status === 'fulfilled' ? workflowRunsResult.value : null
  const workflowRunJobsByRunId = new Map<number, WorkflowRunJobsResponse>()

  if (workflowRuns?.workflow_runs.length) {
    const workflowRunJobResults = await Promise.allSettled(
      workflowRuns.workflow_runs
        .filter(run => run.head_sha === headSha)
        .map(async (run) => {
          const jobs = await fetchWorkflowRunJobs({
            token,
            params: {
              owner,
              repo,
              run_id: run.id,
            },
          })

          return [run.id, jobs] as const
        }),
    )

    for (const result of workflowRunJobResults) {
      if (result.status === 'fulfilled') {
        workflowRunJobsByRunId.set(result.value[0], result.value[1])
      }
    }
  }

  return buildGithubPullRequestChecksSummary({
    pullRequest,
    branchRules: branchRulesResult.status === 'fulfilled' ? branchRulesResult.value : null,
    checkRuns: checkRunsResult.status === 'fulfilled' ? checkRunsResult.value : null,
    workflowRuns,
    combinedStatus: combinedStatusResult.status === 'fulfilled' ? combinedStatusResult.value : null,
    workflowRunJobsByRunId,
  })
}
