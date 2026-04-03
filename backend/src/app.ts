import { Hono } from 'hono'
import { cors } from 'hono/cors'

import { secureHeaders } from 'hono/secure-headers'
import { auth } from './lib/auth.js'
import { logger } from './lib/logger.js'
import { getTrustedOrigins } from './lib/utils.js'
import { loggerMiddleware } from './middlewares/logger.js'
import { routes } from './routes/index.js'

const app = new Hono()

app.use(secureHeaders())
app.use(loggerMiddleware)

app.onError((err, c) => {
  logger.error({ error: err }, 'Unexpected error occurred while handling request')

  return c.json({ message: 'Custom Error Message' }, 500)
})

app.use(
  '*',
  cors({
    origin: getTrustedOrigins(),
    credentials: true,
  }),
)

app.on(['POST', 'GET'], '/api/auth/*', c => auth.handler(c.req.raw))

routes(app)

export { app }
