import type { AuthType } from './lib/auth.ts'

declare module 'hono' {
  interface ContextVariableMap {
    user?: NonNullable<AuthType['user']>
  }
}
