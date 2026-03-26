import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'

import {
  checkDesktopUpdate,
  downloadDesktopUpdateReleaseAsset,
  downloadLatestDesktopUpdateAsset,
  fetchChangelog,
  normalizeSemver,
} from '../plugins/desktop_update/service.js'

const desktopPlatformSchema = z.enum(['macos', 'linux', 'windows'])
const desktopArchSchema = z.enum(['x86_64', 'aarch64'])

const desktopUpdateCheckSchema = z.object({
  currentVersion: z.string().trim().min(1, 'Missing currentVersion'),
  platform: desktopPlatformSchema,
  arch: desktopArchSchema,
})

const desktopUpdateLatestDownloadParamsSchema = z.object({
  platform: desktopPlatformSchema,
  arch: desktopArchSchema,
  fileName: z.string().trim().min(1, 'Missing fileName'),
})

const desktopUpdateReleaseDownloadParamsSchema = z.object({
  tag: z.string().trim().min(1, 'Missing tag'),
  fileName: z.string().trim().min(1, 'Missing fileName'),
})

const router = new Hono()

export const desktopUpdateRoutes = router
  .post('/check', zValidator('json', desktopUpdateCheckSchema), async (ctx) => {
    const { currentVersion, platform, arch } = ctx.req.valid('json')

    const normalizedCurrentVersion = normalizeSemver(currentVersion)

    if (!normalizedCurrentVersion) {
      console.error(`Invalid currentVersion: ${currentVersion}`)
      return ctx.json({ error: 'Invalid currentVersion' }, 400)
    }

    try {
      const result = await checkDesktopUpdate({
        currentVersion: normalizedCurrentVersion,
        platform,
        arch,
      })

      return ctx.json(result, 200)
    }
    catch (error) {
      console.error('Error during desktop update check:', (error as Error).message)
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/download/latest/:platform/:arch/:fileName', async (ctx) => {
    const parsedParams = desktopUpdateLatestDownloadParamsSchema.safeParse(ctx.req.param())

    if (!parsedParams.success) {
      return ctx.json({ error: 'Invalid desktop update download params' }, 400)
    }

    const { platform, arch } = parsedParams.data

    try {
      const asset = await downloadLatestDesktopUpdateAsset({ platform, arch })
      const safeFileName = asset.fileName.replaceAll('"', '')

      return new Response(asset.data, {
        status: 200,
        headers: {
          'Content-Type': asset.contentType,
          'Content-Length': String(asset.size),
          'Content-Disposition': `attachment; filename="${safeFileName}"`,
          'Cache-Control': 'public, max-age=300',
        },
      })
    }
    catch (error) {
      console.error('Error during latest desktop update download:', (error as Error).message)
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/download/release/:tag/:fileName', async (ctx) => {
    const parsedParams = desktopUpdateReleaseDownloadParamsSchema.safeParse(ctx.req.param())

    if (!parsedParams.success) {
      return ctx.json({ error: 'Invalid desktop update release download params' }, 400)
    }

    const { tag, fileName } = parsedParams.data

    try {
      const asset = await downloadDesktopUpdateReleaseAsset(tag, fileName)
      const safeFileName = asset.fileName.replaceAll('"', '')

      return new Response(asset.data, {
        status: 200,
        headers: {
          'Content-Type': asset.contentType,
          'Content-Length': String(asset.size),
          'Content-Disposition': `attachment; filename="${safeFileName}"`,
          'Cache-Control': 'public, max-age=300',
        },
      })
    }
    catch (error) {
      console.error('Error during desktop update release download:', (error as Error).message)
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
  .get('/changelog', async (ctx) => {
    try {
      const entries = await fetchChangelog()
      return ctx.json(entries, 200)
    }
    catch (error) {
      console.error('Error fetching changelog:', (error as Error).message)
      return ctx.json({ error: (error as Error).message }, 502)
    }
  })
