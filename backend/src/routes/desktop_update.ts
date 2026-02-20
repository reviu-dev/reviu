import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'

import {
  checkDesktopUpdate,
  normalizeSemver,
} from '../services/desktop_update.js'

const desktopPlatformSchema = z.enum(['macos', 'linux', 'windows'])
const desktopArchSchema = z.enum(['x86_64', 'aarch64'])

const desktopUpdateCheckSchema = z.object({
  currentVersion: z.string().trim().min(1, 'Missing currentVersion'),
  platform: desktopPlatformSchema,
  arch: desktopArchSchema,
})

const router = new Hono()

export const desktopUpdateRoutes = router.post('/check', zValidator('json', desktopUpdateCheckSchema), async (ctx) => {
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
