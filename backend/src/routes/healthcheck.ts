import { Hono } from 'hono'

type HealthServiceStatus = 'ok' | 'degraded' | 'error'

interface HealthServiceResponse {
  status: HealthServiceStatus
  error?: string
}

interface HealthcheckResponse {
  status: HealthServiceStatus
  services: {
    db: HealthServiceResponse
    redis: HealthServiceResponse
  }
}

interface HealthcheckDependencies {
  checkDatabase: () => Promise<void>
  checkRedis: () => Promise<void>
}

async function checkDatabaseHealth(): Promise<void> {
  const { db } = await import('../db/index.js')
  await db.execute('SELECT 1')
}

async function checkRedisHealth(): Promise<void> {
  const { assertRedisHealthy } = await import('../lib/redis.js')
  await assertRedisHealthy()
}

function toErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message
  }

  return 'Unknown error'
}

export function createHealthcheckRoutes({
  checkDatabase = checkDatabaseHealth,
  checkRedis = checkRedisHealth,
}: Partial<HealthcheckDependencies> = {}) {
  const healthcheckRouter = new Hono()

  return healthcheckRouter.get('/', async (ctx) => {
    const [dbResult, redisResult] = await Promise.allSettled([
      checkDatabase(),
      checkRedis(),
    ])

    const services: HealthcheckResponse['services'] = {
      db: dbResult.status === 'fulfilled'
        ? { status: 'ok' }
        : {
            status: 'error',
            error: toErrorMessage(dbResult.reason),
          },
      redis: redisResult.status === 'fulfilled'
        ? { status: 'ok' }
        : {
            status: 'degraded',
            error: toErrorMessage(redisResult.reason),
          },
    }

    const response: HealthcheckResponse = {
      status: services.db.status === 'error'
        ? 'error'
        : services.redis.status === 'degraded'
          ? 'degraded'
          : 'ok',
      services,
    }

    return ctx.json(response, response.status === 'error' ? 500 : 200)
  })
}

export const healthcheckRoutes = createHealthcheckRoutes()
