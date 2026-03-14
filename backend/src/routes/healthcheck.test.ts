import { describe, expect, it } from 'vitest'
import { createHealthcheckRoutes } from './healthcheck.js'

describe('healthcheck routes', () => {
  it('returns ok when database and redis are healthy', async () => {
    const app = createHealthcheckRoutes({
      checkDatabase: async () => {},
      checkRedis: async () => {},
    })

    const response = await app.request('http://localhost/')

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({
      status: 'ok',
      services: {
        db: { status: 'ok' },
        redis: { status: 'ok' },
      },
    })
  })

  it('returns degraded when redis is unavailable but database is healthy', async () => {
    const app = createHealthcheckRoutes({
      checkDatabase: async () => {},
      checkRedis: async () => {
        throw new Error('Redis unavailable, using in-memory fallback cache')
      },
    })

    const response = await app.request('http://localhost/')

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({
      status: 'degraded',
      services: {
        db: { status: 'ok' },
        redis: {
          status: 'degraded',
          error: 'Redis unavailable, using in-memory fallback cache',
        },
      },
    })
  })

  it('returns error when the database is unavailable', async () => {
    const app = createHealthcheckRoutes({
      checkDatabase: async () => {
        throw new Error('database unavailable')
      },
      checkRedis: async () => {},
    })

    const response = await app.request('http://localhost/')

    expect(response.status).toBe(500)
    await expect(response.json()).resolves.toEqual({
      status: 'error',
      services: {
        db: {
          status: 'error',
          error: 'database unavailable',
        },
        redis: { status: 'ok' },
      },
    })
  })
})
