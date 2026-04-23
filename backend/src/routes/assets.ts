import { Hono } from 'hono'
import { logger } from '../lib/logger.js'
import { authMiddlewarePro } from '../middlewares/auth.js'
import { rateLimitMiddleware } from '../middlewares/rate-limit.js'
import { assetsBaseUrl, assetStorage } from '../plugins/assets/runtime.js'
import { assetUrlFor, AssetValidationError, uploadAsset } from '../plugins/assets/service.js'

const assetsRouter = new Hono()

// Cap uploads per user per minute to avoid someone flooding the mock bucket.
assetsRouter.use('/upload', rateLimitMiddleware({ max: 30, windowSec: 60 }))

export const assetsRoutes = assetsRouter
  .post('/upload', authMiddlewarePro, async (ctx) => {
    let formData: FormData
    try {
      formData = await ctx.req.formData()
    }
    catch (error) {
      logger.warn({ error }, 'Asset upload: failed to parse multipart body')
      return ctx.json({ error: 'Invalid multipart body' }, 400)
    }

    const entry = formData.get('file')
    if (!(entry instanceof File)) {
      return ctx.json({ error: 'Missing `file` field' }, 400)
    }

    try {
      const bytes = new Uint8Array(await entry.arrayBuffer())
      const asset = await uploadAsset(assetStorage, {
        bytes,
        contentType: entry.type,
      })
      return ctx.json({ url: assetUrlFor(assetsBaseUrl(), asset) }, 201)
    }
    catch (error) {
      if (error instanceof AssetValidationError) {
        return ctx.json({ error: error.message }, error.status)
      }
      logger.error({ error }, 'Asset upload: storage put failed')
      return ctx.json({ error: 'Failed to store asset' }, 500)
    }
  })
  // Public read - no auth middleware on purpose. GitHub's camo proxy and any
  // third-party renderer must be able to fetch the asset URL without a session.
  .get('/:id', async (ctx) => {
    const id = ctx.req.param('id')
    try {
      const body = await assetStorage.get(id)
      if (!body) {
        return ctx.json({ error: 'Not found' }, 404)
      }
      // Copy into a tight ArrayBuffer so the Response body matches BodyInit
      // regardless of the Uint8Array's backing buffer type.
      const buffer = body.bytes.buffer.slice(
        body.bytes.byteOffset,
        body.bytes.byteOffset + body.bytes.byteLength,
      ) as ArrayBuffer
      return new Response(buffer, {
        status: 200,
        headers: {
          'Cache-Control': 'public, max-age=31536000, immutable',
          'Content-Type': body.contentType,
          'Content-Length': body.byteLength.toString(),
        },
      })
    }
    catch (error) {
      logger.error({ error, id }, 'Asset fetch failed')
      return ctx.json({ error: 'Failed to fetch asset' }, 500)
    }
  })
