<script setup lang="ts">
import type { AlertVariants } from '@/components/ui/alert'

import { useIntervalFn } from '@vueuse/core'
import { Activity, CircleAlert, CircleCheckBig, LoaderCircle } from 'lucide-vue-next'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { healthcheckClient } from '@/services/healthcheck'

type HealthcheckResponse = Awaited<ReturnType<(typeof healthcheckClient)['index']['$get']>>
type HealthcheckPayload = Awaited<ReturnType<HealthcheckResponse['json']>>

const healthcheck = ref<HealthcheckPayload | null>(null)
const isLoading = ref(false)
const errorMessage = ref<string | null>(null)
const lastFetchedAt = ref<number | null>(null)

let requestId = 0

const timeFormatter = new Intl.DateTimeFormat('en-US', {
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
})

const visualStatus = computed<'ok' | 'degraded' | 'error' | 'loading'>(() => {
  if (isLoading.value && !healthcheck.value) {
    return 'loading'
  }

  if (errorMessage.value) {
    return 'error'
  }

  return healthcheck.value?.status ?? 'loading'
})

const alertVariant = computed<AlertVariants['variant']>(() => {
  const variantMap: Record<string, AlertVariants['variant']> = {
    ok: 'default',
    degraded: 'degraded',
    error: 'destructive',
    loading: 'default',
  }

  return variantMap[visualStatus.value] || 'default'
})

const statusIcon = computed(() => {
  const iconMap: Record<string, any> = {
    ok: CircleCheckBig,
    degraded: CircleAlert,
    error: CircleAlert,
    loading: LoaderCircle,
  }

  return iconMap[visualStatus.value] || LoaderCircle
})

const statusIconClass = computed(() => {
  const classMap: Record<string, string> = {
    ok: 'text-emerald-500!',
    degraded: 'text-amber-500!',
    error: 'text-red-400!',
    loading: 'text-muted-foreground',
  }

  return classMap[visualStatus.value] || 'text-muted-foreground'
})

const title = computed(() => {
  const titleMap: Record<string, string> = {
    ok: 'Healthcheck OK',
    degraded: 'Healthcheck degraded',
    error: 'Healthcheck error',
    loading: 'Checking health',
  }

  return titleMap[visualStatus.value] || 'Checking health'
})

const description = computed(() => {
  if (errorMessage.value) {
    return errorMessage.value
  }

  if (!healthcheck.value) {
    return 'Waiting for backend status.'
  }

  const segments = [
    `DB ${healthcheck.value.services.db.status}`,
    `Redis ${healthcheck.value.services.redis.status}`,
  ]

  return segments.join(' • ')
})

async function fetchHealthcheck() {
  const currentRequestId = ++requestId

  isLoading.value = true
  errorMessage.value = null

  try {
    const response = await healthcheckClient.index.$get()
    const payload = await response.json()

    if (currentRequestId !== requestId) {
      return
    }

    healthcheck.value = payload
    lastFetchedAt.value = Date.now()
  }
  catch (error) {
    if (currentRequestId !== requestId) {
      return
    }

    healthcheck.value = null
    errorMessage.value = error instanceof Error ? error.message : 'Failed to load healthcheck'
  }
  finally {
    if (currentRequestId === requestId) {
      isLoading.value = false
    }
  }
}

const { pause, resume } = useIntervalFn(() => {
  void fetchHealthcheck()
}, 30_000, {
  immediate: false,
})

onMounted(() => {
  void fetchHealthcheck()
  resume()
})

onBeforeUnmount(() => {
  pause()
})
</script>

<template>
  <Alert
    :variant="alertVariant"
    class="border-sidebar-border/70 bg-sidebar-accent/40 text-sidebar-foreground"
  >
    <component
      :is="statusIcon"
      class="mt-0.5"
      :class="[
        statusIconClass,
        visualStatus === 'loading' ? 'animate-spin' : '',
      ]"
    />
    <AlertTitle class="flex items-center gap-2">
      <Activity class="size-4" />
      {{ title }}
    </AlertTitle>
    <AlertDescription class="text-sidebar-foreground/80">
      {{ description }}
      <p v-if="lastFetchedAt">
        {{ `Updated ${timeFormatter.format(lastFetchedAt)}` }}
      </p>
    </AlertDescription>
  </Alert>
</template>
