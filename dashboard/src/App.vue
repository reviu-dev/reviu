<script setup lang="ts">
import { useColorMode } from '@vueuse/core'
import { LoaderCircle } from 'lucide-vue-next'
import { computed, defineAsyncComponent } from 'vue'
import { useRoute } from 'vue-router'

import { useAuthStore } from '@/stores/auth'
import 'vue-sonner/style.css'

useColorMode()
const route = useRoute()
const { firstLoad } = useAuthStore()

const layouts: Record<string, ReturnType<typeof defineAsyncComponent>> = {
  auth: defineAsyncComponent(() => import('@/layouts/AuthLayout.vue')),
  app: defineAsyncComponent(() => import('@/layouts/AppLayout.vue')),
}

const currentLayout = computed(() => route.meta.layout ? layouts[route.meta.layout] : layouts.app)
</script>

<template>
  <Transition
    enter-active-class="transition-opacity duration-300 ease-in"
    leave-active-class="transition-opacity duration-300 ease-out"
    enter-from-class="opacity-0"
    enter-to-class="opacity-100"
    leave-from-class="opacity-100"
    leave-to-class="opacity-0"
  >
    <div v-if="firstLoad" class="fixed inset-0 flex items-center justify-center bg-background z-50">
      <LoaderCircle class="animate-spin size-8 text-gray-600" />
    </div>
  </Transition>
  <component :is="currentLayout" v-if="!firstLoad" />
</template>
