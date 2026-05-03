import { env } from '../../lib/env.js'

import { stowline } from '../../lib/stowline.js'

type FeedbackType = 'bug' | 'feature'
type AppProfile = 'prod' | 'dev'

interface CreateFeedbackParams {
  type: FeedbackType
  title: string
  description: string
  userEmail: string
}

interface CreateCrashReportParams {
  crashId: string
  message: string
  panicLocation?: string
  backtrace?: string
  threadName?: string
  appVersion: string
  release?: string
  os: string
  arch: string
  appProfile: AppProfile
  happenedAt: string
  pathname?: string
  workspacePage?: string
  gitContext?: {
    repoName?: string
    repoHash?: string
    selectedFile?: string | null
    branch?: string
    sidebarMode: string
    diffView: string
  }
  githubPrContext?: {
    owner: string
    repo: string
    number: number
    selectedFile?: string
    activeTab?: number
  }
  userEmail?: string
}

const STOWLINE_TITLE_MAX_CHARS = 180
const STOWLINE_BACKTRACE_MAX_CHARS = 12_000

export async function createFeedbackIssue(params: CreateFeedbackParams) {
  return stowline.issues.create.mutate({
    projectId: env.STOWLINE_PROJECT_ID,
    title: params.title,
    description: params.description,
    labels: ['user-feedback', params.type],
    status: 'backlog',
    metadata: {
      submittedBy: params.userEmail,
    },
  })
}

function trimSingleLine(value: string, maxChars: number) {
  const normalized = value.replace(/\s+/g, ' ').trim()
  if (normalized.length <= maxChars) {
    return normalized
  }
  return `${normalized.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`
}

function trimMultiline(value: string, maxChars: number) {
  const normalized = value.trim()
  if (normalized.length <= maxChars) {
    return normalized
  }
  return `${normalized.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`
}

function buildCrashReportIssueTitle(params: CreateCrashReportParams) {
  return trimSingleLine(
    `Desktop crash on ${params.os}/${params.arch}: ${params.message}`,
    STOWLINE_TITLE_MAX_CHARS,
  )
}

function buildCrashReportIssueDescription(params: CreateCrashReportParams) {
  const sections = [
    '# Desktop Crash Report',
    '',
    `- Crash ID: \`${params.crashId}\``,
    `- Message: ${params.message.trim()}`,
    `- App version: \`${params.appVersion}\``,
    `- Release: \`${params.release?.trim() || 'unknown'}\``,
    `- Profile: \`${params.appProfile}\``,
    `- Platform: \`${params.os}/${params.arch}\``,
    `- Happened at: \`${params.happenedAt}\``,
  ]

  if (params.threadName?.trim()) {
    sections.push(`- Thread: \`${params.threadName.trim()}\``)
  }

  if (params.panicLocation?.trim()) {
    sections.push(`- Panic location: \`${params.panicLocation.trim()}\``)
  }

  if (params.userEmail?.trim()) {
    sections.push(`- Submitted by: \`${params.userEmail.trim()}\``)
  }

  if (params.workspacePage?.trim()) {
    sections.push('', '## UI Context', '')
    sections.push(`- Workspace page: \`${params.workspacePage.trim()}\``)
    if (params.pathname?.trim()) {
      sections.push(`- Pathname: \`${params.pathname.trim()}\``)
    }
  }

  if (params.gitContext) {
    sections.push('', '## Git Context', '')
    if (params.gitContext.repoName?.trim()) {
      sections.push(`- Repo: \`${params.gitContext.repoName.trim()}\``)
    }
    if (params.gitContext.repoHash?.trim()) {
      sections.push(`- Repo hash: \`${params.gitContext.repoHash.trim()}\``)
    }
    if (params.gitContext.branch?.trim()) {
      sections.push(`- Branch: \`${params.gitContext.branch.trim()}\``)
    }
    if (params.gitContext.selectedFile?.trim()) {
      sections.push(`- Selected file: \`${params.gitContext.selectedFile.trim()}\``)
    }
    sections.push(`- Sidebar mode: \`${params.gitContext.sidebarMode.trim()}\``)
    sections.push(`- Diff view: \`${params.gitContext.diffView.trim()}\``)
  }

  if (params.githubPrContext) {
    sections.push('', '## GitHub PR Context', '')
    sections.push(`- Repository: \`${params.githubPrContext.owner}/${params.githubPrContext.repo}\``)
    sections.push(`- Pull request: \`${params.githubPrContext.number}\``)
    if (params.githubPrContext.selectedFile?.trim()) {
      sections.push(`- Selected file: \`${params.githubPrContext.selectedFile.trim()}\``)
    }
    if (params.githubPrContext.activeTab !== undefined) {
      sections.push(`- Active tab: \`${params.githubPrContext.activeTab}\``)
    }
  }

  const backtrace = params.backtrace?.trim()
  if (backtrace) {
    sections.push(
      '',
      '## Backtrace',
      '',
      '```text',
      trimMultiline(backtrace, STOWLINE_BACKTRACE_MAX_CHARS),
      '```',
    )
  }

  return sections.join('\n')
}

export async function createCrashReportIssue(params: CreateCrashReportParams) {
  const labels = [
    'desktop-crash',
    'panic',
    `platform:${params.os}`,
    `profile:${params.appProfile}`,
  ]

  return stowline.issues.create.mutate({
    projectId: env.STOWLINE_PROJECT_ID,
    title: buildCrashReportIssueTitle(params),
    description: buildCrashReportIssueDescription(params),
    labels,
    status: 'backlog',
    metadata: {
      crashId: params.crashId,
      appVersion: params.appVersion,
      release: params.release ?? null,
      platform: params.os,
      arch: params.arch,
      profile: params.appProfile,
      workspacePage: params.workspacePage ?? null,
      pathname: params.pathname ?? null,
      gitRepoHash: params.gitContext?.repoHash ?? null,
      githubRepository: params.githubPrContext
        ? `${params.githubPrContext.owner}/${params.githubPrContext.repo}`
        : null,
      githubPrNumber: params.githubPrContext?.number ?? null,
      submittedBy: params.userEmail ?? null,
    },
  })
}
