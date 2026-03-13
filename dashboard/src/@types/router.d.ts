import type { Layout } from '@/types/layouts'
import 'vue-router'

declare module 'vue-router' {
  interface RouteMeta {
    /**
     *  @default 'app'
     */
    layout?: Layout
  }
}
