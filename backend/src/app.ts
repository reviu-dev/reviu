import { Hono } from 'hono'
import { cors } from 'hono/cors'

import { secureHeaders } from 'hono/secure-headers'
import { auth } from './lib/auth.js'
import { getTrustedOrigins } from './lib/utils.js'
import { routes } from './routes/index.js'

const app = new Hono()

app.use(secureHeaders())

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
