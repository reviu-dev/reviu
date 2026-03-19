import type {
  BranchRulesResponse,
  CommitCheckRunsResponse,
  CommitCombinedStatusResponse,
  PullRequestDetailsResponse,
  WorkflowRunJobsResponse,
  WorkflowRunsResponse,
} from './types.js'
import { describe, expect, it } from 'vitest'
import { buildGithubPullRequestChecksSummary } from './pull-request-checks.js'

function makePullRequest(overrides: Record<string, unknown> = {}): PullRequestDetailsResponse {
  return {
    head: {
      sha: 'head-sha',
    },
    base: {
      ref: 'main',
    },
    ...overrides,
  } as unknown as PullRequestDetailsResponse
}

function makeBranchRules(rules: unknown[]): BranchRulesResponse {
  return rules as BranchRulesResponse
}

function makeCheckRuns(checkRuns: unknown[]): CommitCheckRunsResponse {
  return {
    total_count: checkRuns.length,
    check_runs: checkRuns,
  } as CommitCheckRunsResponse
}

function makeWorkflowRuns(workflowRuns: unknown[]): WorkflowRunsResponse {
  return {
    total_count: workflowRuns.length,
    workflow_runs: workflowRuns,
  } as WorkflowRunsResponse
}

function makeWorkflowJobs(jobs: unknown[]): WorkflowRunJobsResponse {
  return {
    total_count: jobs.length,
    jobs,
  } as WorkflowRunJobsResponse
}

function makeCombinedStatus(statuses: unknown[]): CommitCombinedStatusResponse {
  return {
    state: 'pending',
    statuses,
    sha: 'head-sha',
    total_count: statuses.length,
  } as CommitCombinedStatusResponse
}

describe('pull request checks summary', () => {
  it('summarizes workflow jobs, other checks, and missing required contexts', () => {
    const summary = buildGithubPullRequestChecksSummary({
      pullRequest: makePullRequest(),
      branchRules: makeBranchRules([
        {
          type: 'required_status_checks',
          parameters: {
            strict_required_status_checks_policy: true,
            required_status_checks: [
              { context: 'build' },
              { context: 'lint' },
              { context: 'deploy' },
            ],
          },
        },
      ]),
      workflowRuns: makeWorkflowRuns([
        {
          id: 100,
          name: 'CI',
          display_title: 'CI',
          event: 'pull_request',
          status: 'completed',
          conclusion: 'success',
          created_at: '2026-03-19T10:00:00Z',
          updated_at: '2026-03-19T10:05:00Z',
          run_started_at: '2026-03-19T10:00:30Z',
          run_number: 12,
          run_attempt: 1,
          html_url: 'https://github.com/acme/widget/actions/runs/100',
          head_sha: 'head-sha',
        },
      ]),
      workflowRunJobsByRunId: new Map([
        [100, makeWorkflowJobs([
          {
            id: 200,
            name: 'build',
            status: 'completed',
            conclusion: 'success',
            started_at: '2026-03-19T10:00:30Z',
            completed_at: '2026-03-19T10:02:00Z',
            html_url: 'https://github.com/acme/widget/actions/runs/100/job/200',
            check_run_url: 'https://api.github.com/repos/acme/widget/check-runs/300',
            steps: [
              {
                number: 1,
                name: 'Install',
                status: 'completed',
                conclusion: 'success',
                started_at: '2026-03-19T10:00:31Z',
                completed_at: '2026-03-19T10:00:50Z',
              },
            ],
          },
        ])],
      ]),
      checkRuns: makeCheckRuns([
        {
          id: 300,
          name: 'build',
          status: 'completed',
          conclusion: 'success',
          started_at: '2026-03-19T10:00:30Z',
          completed_at: '2026-03-19T10:02:00Z',
          html_url: 'https://github.com/acme/widget/runs/300',
          details_url: 'https://github.com/acme/widget/runs/300',
          output: {
            title: 'Build',
            summary: 'Build passed',
            text: null,
            annotations_count: 0,
          },
          app: {
            name: 'GitHub Actions',
            slug: 'github-actions',
          },
        },
        {
          id: 301,
          name: 'lint',
          status: 'completed',
          conclusion: 'failure',
          started_at: '2026-03-19T10:01:00Z',
          completed_at: '2026-03-19T10:03:00Z',
          html_url: 'https://github.com/acme/widget/runs/301',
          details_url: 'https://github.com/acme/widget/runs/301',
          output: {
            title: 'Lint',
            summary: 'Lint failed',
            text: 'unused variable',
            annotations_count: 2,
          },
          app: {
            name: 'Reviewdog',
            slug: 'reviewdog',
          },
        },
      ]),
      combinedStatus: makeCombinedStatus([
        {
          id: 401,
          context: 'security/brakeman',
          state: 'success',
          description: 'Security checks passed',
          target_url: 'https://ci.example.com/401',
          created_at: '2026-03-19T10:00:00Z',
          updated_at: '2026-03-19T10:04:00Z',
        },
      ]),
    })

    expect(summary.actions_runs).toHaveLength(1)
    expect(summary.actions_runs[0].jobs).toHaveLength(1)
    expect(summary.actions_runs[0].jobs[0].required).toBe(true)

    expect(summary.other_checks).toHaveLength(1)
    expect(summary.other_checks[0].name).toBe('lint')
    expect(summary.other_checks[0].required).toBe(true)

    expect(summary.legacy_statuses).toHaveLength(1)
    expect(summary.legacy_statuses[0].context).toBe('security/brakeman')

    expect(summary.missing_required_contexts).toEqual(['deploy'])
    expect(summary.requires_up_to_date_branch).toBe(true)
    expect(summary.required_checks_total).toBe(3)
    expect(summary.required_checks_passed).toBe(1)
    expect(summary.required_checks_failed).toBe(1)
    expect(summary.required_checks_pending).toBe(1)
    expect(summary.required_state).toBe('failure')
  })

  it('deduplicates legacy statuses by context and keeps the latest update', () => {
    const summary = buildGithubPullRequestChecksSummary({
      pullRequest: makePullRequest(),
      branchRules: makeBranchRules([
        {
          type: 'required_status_checks',
          parameters: {
            strict_required_status_checks_policy: false,
            required_status_checks: [{ context: 'legacy-ci' }],
          },
        },
      ]),
      combinedStatus: makeCombinedStatus([
        {
          id: 1,
          context: 'legacy-ci',
          state: 'success',
          description: 'older',
          target_url: 'https://ci.example.com/1',
          created_at: '2026-03-19T09:00:00Z',
          updated_at: '2026-03-19T09:01:00Z',
        },
        {
          id: 2,
          context: 'legacy-ci',
          state: 'failure',
          description: 'newer',
          target_url: 'https://ci.example.com/2',
          created_at: '2026-03-19T09:02:00Z',
          updated_at: '2026-03-19T09:03:00Z',
        },
      ]),
    })

    expect(summary.legacy_statuses).toHaveLength(1)
    expect(summary.legacy_statuses[0].id).toBe(2)
    expect(summary.legacy_statuses[0].state).toBe('failure')
    expect(summary.missing_required_contexts).toEqual([])
    expect(summary.required_state).toBe('failure')
  })
})
