import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { auth } from '../lib/auth.js'
import { logger } from '../lib/logger.js'
import { createCrashReportIssue } from '../plugins/feedback/service.js'

const crashReportsRouter = new Hono()

const crashReportBodySchema = z.object({
  crashId: z.string().min(1).max(128),
  message: z.string().min(1).max(2_000),
  panicLocation: z.string().max(1_000).optional(),
  backtrace: z.string().max(40_000).optional(),
  threadName: z.string().max(200).optional(),
  appVersion: z.string().min(1).max(64),
  release: z.string().max(200).optional(),
  os: z.string().min(1).max(64),
  arch: z.string().min(1).max(64),
  appProfile: z.enum(['prod', 'dev']),
  happenedAt: z.string().min(1).max(100),
  pathname: z.string().max(1_000).optional(),
  workspacePage: z.string().max(100).optional(),
  gitContext: z.object({
    repoName: z.string().max(200).optional(),
    repoHash: z.string().max(64).optional(),
    selectedFile: z.string().max(1_000).optional().nullable(),
    branch: z.string().max(200).optional(),
    sidebarMode: z.string().min(1).max(100),
    diffView: z.string().min(1).max(100),
  }).optional(),
  githubPrContext: z.object({
    owner: z.string().min(1).max(200),
    repo: z.string().min(1).max(200),
    number: z.number().int().positive(),
    selectedFile: z.string().max(1_000).optional(),
    activeTab: z.number().int().nonnegative().optional(),
  }).optional(),
})

export const crashReportRoutes = crashReportsRouter
  .post(
    '/',
    zValidator('json', crashReportBodySchema),
    async (ctx) => {
      const payload = ctx.req.valid('json')

      try {
        let userEmail: string | undefined
        try {
          const session = await auth.api.getSession({ headers: ctx.req.raw.headers })
          userEmail = session?.user?.email
        }
        catch (error) {
          logger.warn({ error }, 'Failed to resolve session for crash report')
        }

        const result = await createCrashReportIssue({
          ...payload,
          userEmail,
        })
        return ctx.json({ issueId: result.id }, 201)
      }
      catch (error) {
        logger.error({ error }, 'Failed to create crash report issue')
        return ctx.json({ error: 'Failed to submit crash report' }, 502)
      }
    },
  )
