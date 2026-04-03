import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  buildCrashReportIssueDescription,
  buildCrashReportIssueTitle,
  createCrashReportIssue,
} from './service.js'

const fetchMock = vi.fn()

vi.stubGlobal('fetch', fetchMock)

describe('feedback Shipit service', () => {
  beforeEach(() => {
    fetchMock.mockReset()
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  it('builds a compact crash report title and trims long messages', () => {
    const title = buildCrashReportIssueTitle({
      crashId: 'crash-123',
      message: 'x'.repeat(300),
      appVersion: '0.0.11',
      os: 'macos',
      arch: 'aarch64',
      appProfile: 'prod',
      happenedAt: '2026-04-03T10:00:00Z',
    })

    expect(title).toMatch(/^Desktop crash on macos\/aarch64:/)
    expect(title.length).toBeLessThanOrEqual(180)
    expect(title.endsWith('...')).toBe(true)
  })

  it('creates Shipit issues for desktop crash reports with labels and trimmed backtraces', async () => {
    fetchMock.mockResolvedValue(new Response(JSON.stringify({
      issueId: 'issue-123',
      url: 'https://shipit.example.com/issues/issue-123',
    }), {
      status: 201,
      headers: {
        'content-type': 'application/json',
      },
    }))

    await createCrashReportIssue({
      crashId: 'crash-123',
      message: 'editor repaint panic',
      panicLocation: 'desktop/crates/editor/src/editor.rs:42',
      backtrace: 'frame\n'.repeat(4_000),
      threadName: 'main',
      appVersion: '0.0.11',
      release: 'reviu@0.0.11',
      os: 'macos',
      arch: 'aarch64',
      appProfile: 'prod',
      happenedAt: '2026-04-03T10:00:00Z',
      pathname: '/git',
      workspacePage: 'git',
      gitContext: {
        repoName: 'reviu',
        repoHash: 'abc123def456',
        selectedFile: 'desktop/crates/editor/src/editor.rs',
        branch: 'main',
        sidebarMode: 'changes',
        diffView: 'unified',
      },
      userEmail: 'user@example.com',
    })

    expect(fetchMock).toHaveBeenCalledTimes(1)
    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('https://shipit.example.com/api/v1/issues')
    expect(init.method).toBe('POST')

    const body = JSON.parse(String(init.body))
    expect(body.labels).toEqual([
      'desktop-crash',
      'panic',
      'platform:macos',
      'profile:prod',
    ])
    expect(body.metadata).toEqual({
      crashId: 'crash-123',
      appVersion: '0.0.11',
      release: 'reviu@0.0.11',
      platform: 'macos',
      arch: 'aarch64',
      profile: 'prod',
      workspacePage: 'git',
      pathname: '/git',
      gitRepoHash: 'abc123def456',
      githubRepository: null,
      githubPrNumber: null,
      submittedBy: 'user@example.com',
    })
    expect(body.description).toContain('# Desktop Crash Report')
    expect(body.description).toContain('## UI Context')
    expect(body.description).toContain('## Git Context')
    expect(body.description).toContain('editor repaint panic')
    expect(body.description).toContain('Submitted by')
    expect(body.description.length).toBeLessThan('frame\n'.repeat(4_000).length + 1_000)
  })

  it('builds a readable crash description without optional fields', () => {
    const description = buildCrashReportIssueDescription({
      crashId: 'crash-123',
      message: 'boom',
      appVersion: '0.0.11',
      os: 'linux',
      arch: 'x86_64',
      appProfile: 'dev',
      happenedAt: '2026-04-03T10:00:00Z',
    })

    expect(description).toContain('Crash ID')
    expect(description).toContain('Profile: `dev`')
    expect(description).not.toContain('## Backtrace')
  })
})
