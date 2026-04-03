import { beforeEach, describe, expect, it, vi } from 'vitest'

const getSession = vi.fn()
const createCrashReportIssue = vi.fn()

vi.mock('../../lib/auth.js', () => ({
  auth: {
    api: {
      getSession,
    },
  },
}))

vi.mock('../../plugins/feedback/service.js', () => ({
  createCrashReportIssue,
}))

async function request(path: string, init?: RequestInit) {
  const { crashReportRoutes } = await import('../crash_reports.js')
  return crashReportRoutes.request(`http://localhost${path}`, init)
}

describe('crash report routes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    getSession.mockResolvedValue(null)
  })

  it('accepts anonymous crash reports and forwards the structured payload to Shipit', async () => {
    createCrashReportIssue.mockResolvedValue({
      issueId: 'issue-123',
      url: 'https://shipit.example.com/issues/issue-123',
    })

    const response = await request('/', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        crashId: 'crash-123',
        message: 'editor panic',
        panicLocation: 'desktop/crates/editor/src/editor.rs:42',
        backtrace: 'frame 1\nframe 2',
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
      }),
    })

    expect(response.status).toBe(201)
    expect(createCrashReportIssue).toHaveBeenCalledWith({
      crashId: 'crash-123',
      message: 'editor panic',
      panicLocation: 'desktop/crates/editor/src/editor.rs:42',
      backtrace: 'frame 1\nframe 2',
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
      userEmail: undefined,
    })
    await expect(response.json()).resolves.toEqual({
      issueId: 'issue-123',
      url: 'https://shipit.example.com/issues/issue-123',
    })
  })

  it('includes the authenticated user email when a bearer session exists', async () => {
    getSession.mockResolvedValue({
      user: {
        email: 'user@example.com',
      },
    })
    createCrashReportIssue.mockResolvedValue({
      issueId: 'issue-456',
      url: 'https://shipit.example.com/issues/issue-456',
    })

    const response = await request('/', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'authorization': 'Bearer desktop-token',
      },
      body: JSON.stringify({
        crashId: 'crash-456',
        message: 'renderer panic',
        appVersion: '0.0.11',
        os: 'linux',
        arch: 'x86_64',
        appProfile: 'dev',
        happenedAt: '2026-04-03T10:00:00Z',
      }),
    })

    expect(response.status).toBe(201)
    expect(createCrashReportIssue).toHaveBeenCalledWith(expect.objectContaining({
      crashId: 'crash-456',
      userEmail: 'user@example.com',
    }))
  })

  it('still submits crash reports when session resolution fails', async () => {
    getSession.mockRejectedValue(new Error('session lookup failed'))
    createCrashReportIssue.mockResolvedValue({
      issueId: 'issue-789',
      url: 'https://shipit.example.com/issues/issue-789',
    })

    const response = await request('/', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        crashId: 'crash-789',
        message: 'background panic',
        appVersion: '0.0.11',
        os: 'macos',
        arch: 'aarch64',
        appProfile: 'prod',
        happenedAt: '2026-04-03T10:00:00Z',
      }),
    })

    expect(response.status).toBe(201)
    expect(createCrashReportIssue).toHaveBeenCalledWith(expect.objectContaining({
      crashId: 'crash-789',
      userEmail: undefined,
    }))
  })
})
