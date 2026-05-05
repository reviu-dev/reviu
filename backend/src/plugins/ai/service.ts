import type {
  GithubGraphqlPullRequestCommitNode,
  GithubPullRequestChecksSummary,
  GithubPullRequestConversation,
  GithubPullRequestMergeReadiness,
  PullRequestDetailsResponse,
  PullRequestFileResponse,
} from '../github/types.js'
import type { AiCredentialMode, AiPrBrief, AiPrBriefModelOutput, AiProvider, AiSettingsBody } from './schemas.js'
import { createHash, randomUUID } from 'node:crypto'
import { and, desc, eq } from 'drizzle-orm'
import { db } from '../../db/index.js'
import { aiPrBrief, aiUsageEvent, aiUserSetting } from '../../db/schemas/index.js'
import { fetchGithubPullRequestChecksSummary } from '../github/pull-request-checks.js'
import { fetchGithubPullRequestMergeReadiness } from '../github/pull-request-merge.js'
import {
  fetchGithubPullRequest,
  fetchGithubPullRequestCommitsGraphql,
  fetchGithubPullRequestConversationGraphql,
  fetchGithubPullRequestFilesAllPages,
} from '../github/service.js'
import { apiKeyHint, decryptSecret, encryptSecret } from './crypto.js'
import { generateAiPrBriefWithProvider } from './providers.js'
import { aiPrBriefSchema } from './schemas.js'

const DEFAULT_MODELS: Record<AiProvider, string> = {
  openai: 'gpt-5.4-mini',
  anthropic: 'claude-sonnet-4-6',
}

const MAX_CONTEXT_FILES = 80
const MAX_PATCH_CHARS_PER_FILE = 2800
const MAX_CONTEXT_COMMITS = 80
const MAX_CONTEXT_COMMENTS = 80

interface GithubPrBriefInput {
  userId: string
  githubToken: string
  owner: string
  repo: string
  pullNumber: number
  forceRefresh?: boolean
}

interface AiUserSettingRow {
  userId: string
  credentialMode: string
  provider: string
  model: string
  encryptedApiKey: string
}

function normalizeOwnerRepo(value: string) {
  return value.trim().toLowerCase()
}

function stableJsonHash(value: unknown) {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex')
}

function truncateText(value: string | null | undefined, max: number) {
  const text = value ?? ''
  if (text.length <= max) {
    return text
  }

  return `${text.slice(0, max)}\n...[truncated]`
}

function commitSubject(message: string) {
  return message
    .split('\n')
    .map(line => line.trim())
    .find(Boolean) ?? 'No commit message'
}

function summarizePullRequest(pullRequest: PullRequestDetailsResponse) {
  return {
    number: pullRequest.number,
    title: pullRequest.title,
    state: pullRequest.state,
    draft: Boolean(pullRequest.draft),
    body: truncateText(pullRequest.body, 4000),
    author: pullRequest.user?.login ?? null,
    labels: pullRequest.labels.map(label => label.name),
    assignees: pullRequest.assignees?.map(assignee => assignee.login) ?? [],
    requestedReviewers: pullRequest.requested_reviewers?.map(reviewer => reviewer.login) ?? [],
    base: {
      ref: pullRequest.base.ref,
      sha: pullRequest.base.sha,
    },
    head: {
      ref: pullRequest.head.ref,
      sha: pullRequest.head.sha,
    },
    stats: {
      additions: pullRequest.additions,
      deletions: pullRequest.deletions,
      changedFiles: pullRequest.changed_files,
      commits: pullRequest.commits,
      comments: pullRequest.comments,
      reviewComments: pullRequest.review_comments,
    },
    mergeableState: pullRequest.mergeable_state ?? null,
  }
}

function summarizeFiles(files: PullRequestFileResponse[]) {
  return files.slice(0, MAX_CONTEXT_FILES).map(file => ({
    path: file.filename,
    previousPath: file.previous_filename ?? null,
    status: file.status,
    additions: file.additions,
    deletions: file.deletions,
    changes: file.changes,
    patch: truncateText(file.patch, MAX_PATCH_CHARS_PER_FILE),
  }))
}

function summarizeCommits(commits: GithubGraphqlPullRequestCommitNode[]) {
  return commits.slice(0, MAX_CONTEXT_COMMITS).map(node => ({
    sha: node.commit.oid,
    subject: commitSubject(node.commit.message),
    message: truncateText(node.commit.message, 1200),
    authoredAt: node.commit.authoredDate,
    committedAt: node.commit.committedDate,
    author: node.commit.author?.user?.login ?? node.commit.author?.name ?? null,
  }))
}

function summarizeConversation(conversation: GithubPullRequestConversation) {
  const issueComments = conversation.issue_comments.map(comment => ({
    kind: 'issue_comment',
    id: comment.node_id,
    author: comment.user?.login ?? null,
    body: truncateText(comment.body, 1200),
    createdAt: comment.created_at,
    updatedAt: comment.updated_at,
  }))
  const reviews = conversation.reviews.map(review => ({
    kind: 'review',
    id: review.node_id,
    author: review.user?.login ?? null,
    state: review.state,
    body: truncateText(review.body, 1200),
    submittedAt: review.submitted_at,
  }))
  const reviewComments = conversation.review_comments.map(comment => ({
    kind: 'review_comment',
    id: comment.node_id,
    author: comment.user?.login ?? null,
    path: comment.path,
    body: truncateText(comment.body, 1200),
    createdAt: comment.created_at,
    updatedAt: comment.updated_at,
    replyToId: comment.in_reply_to_id ?? null,
  }))

  return [...issueComments, ...reviews, ...reviewComments]
    .sort((left, right) => (conversationTimestamp(right)).localeCompare(
      conversationTimestamp(left),
    ))
    .slice(0, MAX_CONTEXT_COMMENTS)
}

function conversationTimestamp(item: {
  updatedAt?: string | null
  submittedAt?: string | null
  createdAt?: string | null
}) {
  return item.updatedAt ?? item.submittedAt ?? item.createdAt ?? ''
}

function summarizeChecks(checks: GithubPullRequestChecksSummary) {
  return {
    headSha: checks.head_sha,
    overallState: checks.overall_state,
    requiredState: checks.required_state,
    counts: {
      total: checks.total_checks,
      successful: checks.successful_checks,
      failed: checks.failed_checks,
      pending: checks.pending_checks,
      skipped: checks.skipped_checks,
      requiredTotal: checks.required_checks_total,
      requiredPassed: checks.required_checks_passed,
      requiredFailed: checks.required_checks_failed,
      requiredPending: checks.required_checks_pending,
      requiredSkipped: checks.required_checks_skipped,
    },
    requiredContexts: checks.required_contexts,
    missingRequiredContexts: checks.missing_required_contexts,
    actionsRuns: checks.actions_runs.map(run => ({
      name: run.name,
      status: run.status,
      conclusion: run.conclusion,
      htmlUrl: run.html_url,
    })),
    otherChecks: checks.other_checks.map(check => ({
      name: check.name,
      state: check.state,
      status: check.status,
      conclusion: check.conclusion,
      htmlUrl: check.html_url,
      required: check.required,
    })),
    legacyStatuses: checks.legacy_statuses.map(status => ({
      context: status.context,
      state: status.state,
      targetUrl: status.target_url,
      required: status.required,
    })),
  }
}

function summarizeMergeReadiness(mergeReadiness: GithubPullRequestMergeReadiness) {
  return {
    status: mergeReadiness.status,
    message: mergeReadiness.message,
    canMergeNow: mergeReadiness.can_merge_now,
    currentHeadSha: mergeReadiness.current_head_sha,
    availableMethods: mergeReadiness.available_methods,
    mergeableState: mergeReadiness.mergeable_state,
    rebaseable: mergeReadiness.rebaseable,
    autoMergeEnabled: mergeReadiness.auto_merge_enabled,
  }
}

function resolveModelReferences(
  output: AiPrBriefModelOutput,
  files: PullRequestFileResponse[],
) {
  const validPaths = new Set(files.flatMap(file => [file.filename, file.previous_filename].filter(Boolean) as string[]))
  return {
    ...output,
    reviewFirst: output.reviewFirst
      .filter(item => validPaths.has(item.path))
      .map(item => ({
        ...item,
        target: {
          type: 'pr_file' as const,
          path: item.path,
        },
      })),
    risks: output.risks.map(risk => ({
      ...risk,
      path: risk.path && validPaths.has(risk.path) ? risk.path : undefined,
      target: risk.path && validPaths.has(risk.path)
        ? {
            type: 'pr_file' as const,
            path: risk.path,
          }
        : null,
    })),
  }
}

async function getAiSettingsRow(userId: string): Promise<AiUserSettingRow | null> {
  const setting = await db.query.aiUserSetting.findFirst({
    where: eq(aiUserSetting.userId, userId),
  })

  return setting ?? null
}

function parseAiSetting(row: AiUserSettingRow) {
  if (row.credentialMode !== 'user_key') {
    throw Object.assign(new Error('Only user-owned AI keys are supported right now.'), { status: 422 })
  }
  if (row.provider !== 'openai' && row.provider !== 'anthropic') {
    throw Object.assign(new Error('Unsupported AI provider.'), { status: 422 })
  }

  return {
    credentialMode: row.credentialMode as AiCredentialMode,
    provider: row.provider as AiProvider,
    model: row.model,
    apiKey: decryptSecret(row.encryptedApiKey),
  }
}

export async function getAiSettings(userId: string) {
  const setting = await getAiSettingsRow(userId)
  if (!setting) {
    return { configured: false as const }
  }

  return {
    configured: true as const,
    credentialMode: setting.credentialMode,
    provider: setting.provider,
    model: setting.model,
    apiKeyHint: apiKeyHint(decryptSecret(setting.encryptedApiKey)),
  }
}

export async function saveAiSettings(userId: string, body: AiSettingsBody) {
  const provider = body.provider
  const model = body.model ?? DEFAULT_MODELS[provider]
  const now = new Date()
  const payload = {
    userId,
    credentialMode: 'user_key',
    provider,
    model,
    encryptedApiKey: encryptSecret(body.apiKey),
    updatedAt: now,
  }

  await db
    .insert(aiUserSetting)
    .values({ ...payload, createdAt: now })
    .onConflictDoUpdate({
      target: aiUserSetting.userId,
      set: payload,
    })

  return getAiSettings(userId)
}

export async function deleteAiSettings(userId: string) {
  await db.delete(aiUserSetting).where(eq(aiUserSetting.userId, userId))
}

export async function getLatestPrBrief(
  userId: string,
  owner: string,
  repo: string,
  pullNumber: number,
): Promise<AiPrBrief | null> {
  const normalizedOwner = normalizeOwnerRepo(owner)
  const normalizedRepo = normalizeOwnerRepo(repo)
  const row = await db.query.aiPrBrief.findFirst({
    where: and(
      eq(aiPrBrief.userId, userId),
      eq(aiPrBrief.owner, normalizedOwner),
      eq(aiPrBrief.repo, normalizedRepo),
      eq(aiPrBrief.pullNumber, pullNumber),
    ),
    orderBy: desc(aiPrBrief.createdAt),
  })

  if (!row) {
    return null
  }

  return aiPrBriefSchema.parse({
    ...(row.briefJson as object),
    provider: row.provider,
    credentialMode: row.credentialMode,
    model: row.model,
    cached: true,
  })
}

async function loadCachedBrief(
  userId: string,
  owner: string,
  repo: string,
  pullNumber: number,
  headSha: string,
  contextHash: string,
) {
  const cached = await db.query.aiPrBrief.findFirst({
    where: and(
      eq(aiPrBrief.userId, userId),
      eq(aiPrBrief.owner, owner),
      eq(aiPrBrief.repo, repo),
      eq(aiPrBrief.pullNumber, pullNumber),
      eq(aiPrBrief.headSha, headSha),
      eq(aiPrBrief.contextHash, contextHash),
    ),
    orderBy: desc(aiPrBrief.createdAt),
  })

  if (!cached) {
    return null
  }

  return aiPrBriefSchema.parse({
    ...(cached.briefJson as object),
    provider: cached.provider,
    credentialMode: cached.credentialMode,
    model: cached.model,
    cached: true,
  })
}

async function saveBrief(userId: string, brief: AiPrBrief) {
  const now = new Date()
  await db
    .insert(aiPrBrief)
    .values({
      id: randomUUID(),
      userId,
      owner: brief.owner,
      repo: brief.repo,
      pullNumber: brief.pullNumber,
      headSha: brief.headSha,
      contextHash: brief.contextHash,
      provider: brief.provider,
      credentialMode: brief.credentialMode,
      model: brief.model,
      briefJson: brief,
      createdAt: now,
      updatedAt: now,
    })
    .onConflictDoUpdate({
      target: [
        aiPrBrief.userId,
        aiPrBrief.owner,
        aiPrBrief.repo,
        aiPrBrief.pullNumber,
        aiPrBrief.headSha,
        aiPrBrief.contextHash,
      ],
      set: {
        provider: brief.provider,
        credentialMode: brief.credentialMode,
        model: brief.model,
        briefJson: brief,
        updatedAt: now,
      },
    })
}

async function recordAiUsage(
  userId: string,
  provider: AiProvider,
  credentialMode: AiCredentialMode,
  model: string,
  usage: { inputTokens: number | null, outputTokens: number | null },
  owner: string,
  repo: string,
  pullNumber: number,
) {
  await db.insert(aiUsageEvent).values({
    id: randomUUID(),
    userId,
    task: 'github.pr.brief',
    provider,
    credentialMode,
    model,
    inputTokens: usage.inputTokens,
    outputTokens: usage.outputTokens,
    owner,
    repo,
    pullNumber,
  })
}

function systemPrompt() {
  return [
    'You generate concise pull request review briefs for Reviu, a desktop Git client.',
    'Keep human judgment with the reviewer. Do not claim the PR is correct.',
    'Use only the provided context. Do not invent files, checks, comments, or risks.',
    'Prefer concrete review guidance over generic software advice.',
    'Return short strings. File paths must be repository-relative paths from the provided file list.',
  ].join('\n')
}

function userPrompt(context: unknown) {
  return [
    'Create an AI PR Brief JSON object for this pull request.',
    'The brief should help a reviewer decide where to start and what might block merge.',
    'Schema fields: summary, reviewFirst, risks, blockers.',
    'Context:',
    JSON.stringify(context),
  ].join('\n\n')
}

export async function generateGithubPrBrief({
  userId,
  githubToken,
  owner,
  repo,
  pullNumber,
  forceRefresh = false,
}: GithubPrBriefInput): Promise<AiPrBrief> {
  const normalizedOwner = normalizeOwnerRepo(owner)
  const normalizedRepo = normalizeOwnerRepo(repo)
  const settingRow = await getAiSettingsRow(userId)

  if (!settingRow) {
    throw Object.assign(new Error('AI settings are not configured.'), { status: 409 })
  }

  const setting = parseAiSetting(settingRow)
  const params = {
    owner: normalizedOwner,
    repo: normalizedRepo,
    pull_number: pullNumber,
  }

  const [pullRequest, files, commits, conversation, checks, mergeReadiness] = await Promise.all([
    fetchGithubPullRequest({ token: githubToken, params }),
    fetchGithubPullRequestFilesAllPages({ token: githubToken, params }),
    fetchGithubPullRequestCommitsGraphql({ token: githubToken, params }),
    fetchGithubPullRequestConversationGraphql({
      token: githubToken,
      owner: normalizedOwner,
      repo: normalizedRepo,
      pullNumber,
    }),
    fetchGithubPullRequestChecksSummary({ token: githubToken, params }),
    fetchGithubPullRequestMergeReadiness({ token: githubToken, params }),
  ])

  const context = {
    pullRequest: summarizePullRequest(pullRequest),
    files: summarizeFiles(files),
    commits: summarizeCommits(commits),
    conversation: summarizeConversation(conversation),
    checks: summarizeChecks(checks),
    mergeReadiness: summarizeMergeReadiness(mergeReadiness),
  }
  const contextHash = stableJsonHash(context)
  const cached = !forceRefresh
    ? await loadCachedBrief(
        userId,
        normalizedOwner,
        normalizedRepo,
        pullNumber,
        pullRequest.head.sha,
        contextHash,
      )
    : null

  if (cached) {
    return cached
  }

  const generated = await generateAiPrBriefWithProvider({
    provider: setting.provider,
    apiKey: setting.apiKey,
    model: setting.model,
    systemPrompt: systemPrompt(),
    userPrompt: userPrompt(context),
  })
  const resolved = resolveModelReferences(generated.output, files)
  const brief = aiPrBriefSchema.parse({
    ...resolved,
    generatedAt: new Date().toISOString(),
    owner: normalizedOwner,
    repo: normalizedRepo,
    pullNumber,
    headSha: pullRequest.head.sha,
    contextHash,
    provider: setting.provider,
    credentialMode: setting.credentialMode,
    model: setting.model,
    cached: false,
  })

  await saveBrief(userId, brief)
  await recordAiUsage(
    userId,
    setting.provider,
    setting.credentialMode,
    setting.model,
    generated.usage,
    normalizedOwner,
    normalizedRepo,
    pullNumber,
  )

  return brief
}
