import type { UserContext } from './lib/auth.ts'

declare module 'hono' {
  interface ContextVariableMap {
    user?: UserContext
  }
}
