<script setup lang="ts">
import type { ChartConfig } from '@/components/ui/chart'
import { VisAxis, VisGroupedBar, VisLine, VisXYContainer } from '@unovis/vue'
import { RefreshCw } from 'lucide-vue-next'
import { computed, ref, watch } from 'vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  ChartContainer,
  ChartCrosshair,
  ChartTooltip,
  ChartTooltipContent,
  componentToString,
} from '@/components/ui/chart'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { adminClient } from '@/services/admin'

type AdminOverviewResponse = Awaited<ReturnType<(typeof adminClient)['github-cache']['overview']['$get']>>
type AdminDrilldownResponse = Awaited<ReturnType<(typeof adminClient)['github-cache']['drilldown']['$get']>>
type GithubCacheOverview = Awaited<ReturnType<AdminOverviewResponse['json']>>
type GithubCacheDrilldown = Awaited<ReturnType<AdminDrilldownResponse['json']>>
type CacheChartRow = GithubCacheOverview['cacheStatusSeries'][number] & { date: Date }
type ScopeChartRow = { date: Date } & Record<string, number | Date>
type ResourceChartRow = { date: Date } & Record<string, number | Date>
type DrilldownChartRow = GithubCacheDrilldown['series'][number] & { date: Date }
type RouteEfficiencyRow = GithubCacheOverview['routes'][number] & {
  upstreamRate: number
  savedRate: number
}

const WINDOW_OPTIONS = [
  { label: '15 min', value: '15' },
  { label: '1 hr', value: '60' },
  { label: '6 hr', value: '360' },
  { label: '24 hr', value: '1440' },
]

const CHART_PALETTE = [
  'var(--chart-1)',
  'var(--chart-2)',
  'var(--chart-3)',
  'var(--chart-4)',
  'var(--chart-5)',
]

const selectedWindowMinutes = ref('60')
const overview = ref<GithubCacheOverview | null>(null)
const isLoading = ref(false)
const errorMessage = ref<string | null>(null)
const lastFetchedAt = ref<number | null>(null)
const drilldown = ref<GithubCacheDrilldown | null>(null)
const isDrilldownLoading = ref(false)
const drilldownErrorMessage = ref<string | null>(null)
const selectedOperation = ref('')
const selectedScope = ref<'all' | GithubCacheOverview['scopeSummary'][number]['scope']>('all')

let requestId = 0
let drilldownRequestId = 0

const compactNumberFormatter = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 1,
})

const averageNumberFormatter = new Intl.NumberFormat('en-US', {
  maximumFractionDigits: 1,
})

const percentFormatter = new Intl.NumberFormat('en-US', {
  style: 'percent',
  maximumFractionDigits: 1,
})

const timeFormatter = new Intl.DateTimeFormat('en-US', {
  hour: '2-digit',
  minute: '2-digit',
})

const dateTimeFormatter = new Intl.DateTimeFormat('en-US', {
  day: '2-digit',
  month: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
})

const cacheChartConfig = {
  hit: {
    label: 'Hit',
    color: 'var(--chart-2)',
  },
  stale: {
    label: 'Stale',
    color: 'var(--chart-4)',
  },
  miss: {
    label: 'Miss',
    color: 'var(--chart-5)',
  },
} satisfies ChartConfig

const drilldownChartConfig = {
  hit: {
    label: 'Hit',
    color: 'var(--chart-2)',
  },
  stale: {
    label: 'Stale',
    color: 'var(--chart-4)',
  },
  miss: {
    label: 'Miss',
    color: 'var(--chart-5)',
  },
  upstreamCalls: {
    label: 'Upstream',
    color: 'var(--chart-1)',
  },
} satisfies ChartConfig

async function fetchOverview() {
  const currentRequestId = ++requestId

  isLoading.value = true
  errorMessage.value = null

  try {
    const response = await adminClient['github-cache'].overview.$get({
      query: {
        windowMinutes: String(Number(selectedWindowMinutes.value)),
        limit: '20',
      },
    })

    if (!response.ok) {
      const payload = await response.json().catch(() => null) as { error?: string, message?: string } | null
      throw new Error(payload?.error ?? payload?.message ?? 'Failed to load GitHub cache overview')
    }

    const data = await response.json()
    if (currentRequestId !== requestId) {
      return
    }

    overview.value = data
    lastFetchedAt.value = Date.now()
  }
  catch (error) {
    if (currentRequestId !== requestId) {
      return
    }

    errorMessage.value = error instanceof Error ? error.message : 'Unknown error'
  }
  finally {
    if (currentRequestId === requestId) {
      isLoading.value = false
    }
  }
}

async function fetchDrilldown() {
  if (!selectedOperation.value) {
    drilldown.value = null
    drilldownErrorMessage.value = null
    return
  }

  const currentRequestId = ++drilldownRequestId

  isDrilldownLoading.value = true
  drilldownErrorMessage.value = null

  try {
    const response = await adminClient['github-cache'].drilldown.$get({
      query: {
        windowMinutes: String(Number(selectedWindowMinutes.value)),
        operation: selectedOperation.value,
        ...(selectedScope.value === 'all' ? {} : { scope: selectedScope.value }),
      },
    })

    if (!response.ok) {
      const payload = await response.json().catch(() => null) as { error?: string, message?: string } | null
      throw new Error(payload?.error ?? payload?.message ?? 'Failed to load GitHub cache drilldown')
    }

    const data = await response.json()
    if (currentRequestId !== drilldownRequestId) {
      return
    }

    drilldown.value = data
  }
  catch (error) {
    if (currentRequestId !== drilldownRequestId) {
      return
    }

    drilldown.value = null
    drilldownErrorMessage.value = error instanceof Error ? error.message : 'Unknown error'
  }
  finally {
    if (currentRequestId === drilldownRequestId) {
      isDrilldownLoading.value = false
    }
  }
}

watch(selectedWindowMinutes, () => {
  fetchOverview()
}, { immediate: true })

watch([selectedWindowMinutes, selectedOperation, selectedScope], () => {
  void fetchDrilldown()
})

const cacheChartData = computed<CacheChartRow[]>(() => {
  return (overview.value?.cacheStatusSeries ?? []).map(item => ({
    ...item,
    date: new Date(item.bucketStart),
  }))
})

const resourceKeys = computed(() => {
  const totals = new Map<string, number>()

  for (const item of overview.value?.githubResourceSeries ?? []) {
    totals.set(item.resource, (totals.get(item.resource) ?? 0) + item.upstreamCalls)
  }

  return [...totals.entries()].toSorted((a, b) => b[1] - a[1]).slice(0, 4).map(([resource]) => resource)
})

const resourceChartConfig = computed<ChartConfig>(() => {
  return Object.fromEntries(
    resourceKeys.value.map((resource, index) => [
      resource,
      {
        label: resource,
        color: CHART_PALETTE[index % CHART_PALETTE.length],
      },
    ]),
  )
})

const resourceChartData = computed<ResourceChartRow[]>(() => {
  const rows = new Map<number, ResourceChartRow>()

  for (const item of overview.value?.githubResourceSeries ?? []) {
    if (!resourceKeys.value.includes(item.resource)) {
      continue
    }

    const current = rows.get(item.bucketStart) ?? {
      date: new Date(item.bucketStart),
    }

    current[item.resource] = Number(current[item.resource] ?? 0) + item.upstreamCalls
    rows.set(item.bucketStart, current)
  }

  return [...rows.entries()].toSorted((a, b) => a[0] - b[0]).map(([, value]) => value)
})

const scopeKeys = computed(() => {
  return (overview.value?.scopeSummary ?? []).map(item => item.scope)
})

const scopeTrafficChartConfig = computed<ChartConfig>(() => {
  return Object.fromEntries(
    scopeKeys.value.map((scope, index) => [
      scope,
      {
        label: formatScopeLabel(scope),
        color: CHART_PALETTE[index % CHART_PALETTE.length],
      },
    ]),
  )
})

const scopeTrafficChartData = computed<ScopeChartRow[]>(() => {
  const rows = new Map<number, ScopeChartRow>()

  for (const item of overview.value?.scopeSeries ?? []) {
    const current = rows.get(item.bucketStart) ?? {
      date: new Date(item.bucketStart),
    }

    current[item.scope] = Number(current[item.scope] ?? 0) + item.requests
    rows.set(item.bucketStart, current)
  }

  return [...rows.entries()].toSorted((a, b) => a[0] - b[0]).map(([, value]) => value)
})

const drilldownChartData = computed<DrilldownChartRow[]>(() => {
  return (drilldown.value?.series ?? []).map(item => ({
    ...item,
    date: new Date(item.bucketStart),
  }))
})

const summaryCards = computed(() => {
  const summary = overview.value?.summary
  if (!summary) {
    return []
  }

  return [
    {
      label: 'Requests',
      value: formatCount(summary.requests),
      detail: `${summary.upstreamCalls} upstream`,
    },
    {
      label: 'Cache hit rate',
      value: formatPercent(summary.hitRate),
      detail: `${formatPercent(summary.staleRate)} stale`,
    },
    {
      label: 'GitHub saved',
      value: formatCount(summary.githubCallsSaved),
      detail: `${summary.notModified} revalidated`,
    },
    {
      label: 'Users near limit',
      value: formatCount(summary.usersNearLimit),
      detail: `${summary.nearLimitEvents} alerts`,
    },
  ]
})

const paginationCards = computed(() => {
  const summary = overview.value?.summary
  if (!summary) {
    return []
  }

  const truncationRate = summary.paginatedLoads > 0
    ? summary.truncatedCount / summary.paginatedLoads
    : null

  return [
    {
      label: 'Paginated rebuilds',
      value: formatCount(summary.paginatedLoads),
      detail: `${formatDuration(summary.avgPaginationDurationMs)} average rebuild`,
    },
    {
      label: 'Average pages',
      value: formatAverage(summary.avgPageCount),
      detail: `${formatAverage(summary.avgItemCount)} average items`,
    },
    {
      label: 'Truncated rebuilds',
      value: formatCount(summary.truncatedCount),
      detail: truncationRate == null ? 'No paginated loads' : `${formatPercent(truncationRate)} of paginated loads`,
    },
    {
      label: 'Pagination headroom',
      value: summary.truncatedCount > 0 ? 'Capped' : 'Clear',
      detail: summary.truncatedCount > 0
        ? 'At least one aggregate hit the configured cap'
        : 'No aggregate hit the configured cap',
    },
  ]
})

const scopeCards = computed(() => overview.value?.scopeSummary ?? [])

const drilldownOperationOptions = computed(() => {
  const options = new Set<string>()

  for (const route of overview.value?.routes ?? []) {
    options.add(route.operation)
  }

  return [...options].toSorted((left, right) => left.localeCompare(right))
})

const drilldownScopeOptions = computed(() => {
  if (!selectedOperation.value) {
    return []
  }

  const scopes = new Set<GithubCacheOverview['scopeSummary'][number]['scope']>()

  for (const route of overview.value?.routes ?? []) {
    if (route.operation === selectedOperation.value && route.scope) {
      scopes.add(route.scope)
    }
  }

  return [...scopes].toSorted((left, right) => left.localeCompare(right))
})

const drilldownSummaryCards = computed(() => {
  const summary = drilldown.value?.summary
  if (!summary) {
    return []
  }

  return [
    {
      label: 'Requests',
      value: formatCount(summary.requests),
      detail: `${formatCount(summary.upstreamCalls)} upstream`,
    },
    {
      label: 'Hit rate',
      value: formatPercent(summary.hitRate),
      detail: `${formatPercent(summary.notModifiedRate)} 304 rate`,
    },
    {
      label: 'Paginated rebuilds',
      value: formatCount(summary.paginatedLoads),
      detail: `${formatCount(summary.truncatedCount)} truncated`,
    },
    {
      label: 'Policy',
      value: formatDuration(summary.ttlMs),
      detail: `stale ${formatDuration(summary.staleMs)}`,
    },
  ]
})

const paginatedRoutes = computed(() => {
  return [...(overview.value?.routes ?? [])]
    .filter(route => route.paginatedLoads > 0)
    .toSorted((left, right) => {
      return right.truncatedCount - left.truncatedCount
        || (right.avgItemCount ?? 0) - (left.avgItemCount ?? 0)
        || right.paginatedLoads - left.paginatedLoads
        || left.operation.localeCompare(right.operation)
    })
})

const truncatedRoutes = computed(() => {
  return paginatedRoutes.value.filter(route => route.truncatedCount > 0)
})

const upstreamHeavyRoutes = computed<RouteEfficiencyRow[]>(() => {
  return [...(overview.value?.routes ?? [])]
    .filter(route => route.requests > 0)
    .map(route => ({
      ...route,
      upstreamRate: route.upstreamCalls / route.requests,
      savedRate: route.githubCallsSaved / route.requests,
    }))
    .toSorted((left, right) => {
      return right.upstreamRate - left.upstreamRate
        || right.upstreamCalls - left.upstreamCalls
        || left.hitRate - right.hitRate
        || right.truncatedCount - left.truncatedCount
        || left.operation.localeCompare(right.operation)
    })
    .slice(0, 8)
})

function syncDrilldownSelection() {
  const routes = overview.value?.routes ?? []

  if (routes.length === 0) {
    selectedOperation.value = ''
    selectedScope.value = 'all'
    drilldown.value = null
    return
  }

  if (!selectedOperation.value || !drilldownOperationOptions.value.includes(selectedOperation.value)) {
    selectedOperation.value = routes[0]?.operation ?? ''
  }

  if (selectedScope.value !== 'all' && !drilldownScopeOptions.value.includes(selectedScope.value)) {
    selectedScope.value = 'all'
  }
}

watch(() => overview.value?.routes, () => {
  syncDrilldownSelection()
}, { immediate: true })

watch(selectedOperation, () => {
  if (selectedScope.value !== 'all' && !drilldownScopeOptions.value.includes(selectedScope.value)) {
    selectedScope.value = 'all'
  }
})

function selectRouteDrilldown(
  operation: string,
  scope: GithubCacheOverview['routes'][number]['scope'],
) {
  selectedOperation.value = operation
  selectedScope.value = scope ?? 'all'
}

function isSelectedRoute(
  operation: string,
  scope: GithubCacheOverview['routes'][number]['scope'],
) {
  return selectedOperation.value === operation
    && selectedScope.value !== 'all'
    && selectedScope.value === (scope ?? 'all')
}

function formatCount(value: number | null | undefined) {
  if (value == null) {
    return '—'
  }

  return compactNumberFormatter.format(value)
}

function formatAverage(value: number | null | undefined) {
  if (value == null) {
    return '—'
  }

  return averageNumberFormatter.format(value)
}

function formatPercent(value: number | null | undefined) {
  if (value == null) {
    return '—'
  }

  return percentFormatter.format(value)
}

function formatDuration(value: number | null | undefined) {
  if (value == null) {
    return '—'
  }

  if (value < 1_000) {
    return `${Math.round(value)} ms`
  }

  return `${(value / 1_000).toFixed(2)} s`
}

function formatRateLimitRemaining(remaining: number | null, limit: number | null) {
  if (remaining == null || limit == null) {
    return '—'
  }

  return `${remaining.toLocaleString('en-US')} / ${limit.toLocaleString('en-US')}`
}

function formatReset(reset: number | null | undefined) {
  if (reset == null) {
    return '—'
  }

  return timeFormatter.format(new Date(reset * 1000))
}

function formatDateTime(value: number | null | undefined) {
  if (value == null) {
    return '—'
  }

  return dateTimeFormatter.format(new Date(value))
}

function formatBucketLabel(value: number | Date) {
  const date = value instanceof Date ? value : new Date(value)
  return timeFormatter.format(date)
}

function pressureVariant(remainingPct: number | null | undefined) {
  if (remainingPct == null) {
    return 'outline'
  }

  if (remainingPct < 0.1) {
    return 'destructive'
  }

  if (remainingPct < 0.25) {
    return 'secondary'
  }

  return 'outline'
}

function scopeVariant(scope: GithubCacheOverview['scopeSummary'][number]['scope']) {
  if (scope === 'public') {
    return 'default'
  }

  if (scope === 'viewer') {
    return 'secondary'
  }

  return 'outline'
}

function formatScopeLabel(scope: GithubCacheOverview['scopeSummary'][number]['scope']) {
  if (scope === 'public') {
    return 'Public scope'
  }

  if (scope === 'viewer') {
    return 'Viewer scope'
  }

  return 'Installation scope'
}
</script>

<template>
  <div class="container mx-auto space-y-6">
    <div class="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
      <div class="space-y-1">
        <h1 class="text-3xl font-bold tracking-tight">
          GitHub Cache
        </h1>
        <p class="text-muted-foreground max-w-3xl">
          Admin view for tracking GitHub pressure, cache hit rates, and the routes that need more aggressive TTL tuning.
        </p>
      </div>

      <div class="flex flex-wrap items-end gap-3">
        <div class="space-y-1">
          <div class="text-muted-foreground text-xs font-medium uppercase tracking-[0.18em]">
            Window
          </div>
          <Select v-model="selectedWindowMinutes">
            <SelectTrigger class="w-[120px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem
                v-for="option in WINDOW_OPTIONS"
                :key="option.value"
                :value="option.value"
              >
                {{ option.label }}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>

        <Button
          variant="outline"
          class="gap-2"
          :disabled="isLoading"
          @click="fetchOverview"
        >
          <RefreshCw class="size-4" :class="{ 'animate-spin': isLoading }" />
          Refresh
        </Button>
      </div>
    </div>

    <div class="flex flex-wrap items-center gap-3 text-sm">
      <Badge variant="outline">
        {{ overview?.bucketMs ? `${overview.bucketMs / 60_000} min / bucket` : '1 min / bucket' }}
      </Badge>
      <Badge v-if="lastFetchedAt" variant="outline">
        Updated {{ formatDateTime(lastFetchedAt) }}
      </Badge>
      <Badge v-if="errorMessage" variant="destructive">
        {{ errorMessage }}
      </Badge>
    </div>

    <div
      v-if="!overview && isLoading"
      class="grid gap-4 md:grid-cols-2 2xl:grid-cols-4"
    >
      <Skeleton
        v-for="index in 4"
        :key="index"
        class="h-32"
      />
    </div>

    <div
      v-if="!overview && isLoading"
      class="grid gap-4 xl:grid-cols-2"
    >
      <Skeleton class="h-[360px]" />
      <Skeleton class="h-[360px]" />
    </div>

    <template v-if="overview">
      <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <Card
          v-for="card in summaryCards"
          :key="card.label"
          class="border-border/60 bg-card/80 backdrop-blur"
        >
          <CardHeader class="pb-2">
            <CardDescription class="text-xs uppercase tracking-[0.18em]">
              {{ card.label }}
            </CardDescription>
            <CardTitle class="text-3xl">
              {{ card.value }}
            </CardTitle>
          </CardHeader>
          <CardContent class="text-muted-foreground text-sm">
            {{ card.detail }}
          </CardContent>
        </Card>
      </div>

      <div
        v-if="scopeCards.length > 0"
        class="grid gap-4 xl:grid-cols-2"
      >
        <Card
          v-for="scope in scopeCards"
          :key="scope.scope"
          class="border-border/60 bg-card/80 backdrop-blur"
        >
          <CardHeader class="pb-2">
            <div class="flex items-center justify-between gap-3">
              <CardTitle class="text-base">
                {{ formatScopeLabel(scope.scope) }}
              </CardTitle>
              <Badge :variant="scopeVariant(scope.scope)">
                {{ scope.scope }}
              </Badge>
            </div>
            <CardDescription>
              {{ formatCount(scope.requests) }} requests, {{ formatCount(scope.upstreamCalls) }} upstream calls
            </CardDescription>
          </CardHeader>
          <CardContent class="space-y-4">
            <div class="grid gap-3 sm:grid-cols-3">
              <div class="space-y-1">
                <div class="text-muted-foreground text-xs uppercase tracking-[0.18em]">
                  Hit rate
                </div>
                <div class="text-xl font-semibold">
                  {{ formatPercent(scope.hitRate) }}
                </div>
              </div>
              <div class="space-y-1">
                <div class="text-muted-foreground text-xs uppercase tracking-[0.18em]">
                  Saved
                </div>
                <div class="text-xl font-semibold">
                  {{ formatCount(scope.githubCallsSaved) }}
                </div>
              </div>
              <div class="space-y-1">
                <div class="text-muted-foreground text-xs uppercase tracking-[0.18em]">
                  GitHub avg
                </div>
                <div class="text-xl font-semibold">
                  {{ formatDuration(scope.avgGithubDurationMs) }}
                </div>
              </div>
            </div>

            <div class="border-border/60 grid gap-3 border-t pt-4 sm:grid-cols-3">
              <div class="space-y-1">
                <div class="text-muted-foreground text-xs uppercase tracking-[0.18em]">
                  Paginated
                </div>
                <div class="text-base font-semibold">
                  {{ formatCount(scope.paginatedLoads) }}
                </div>
              </div>
              <div class="space-y-1">
                <div class="text-muted-foreground text-xs uppercase tracking-[0.18em]">
                  Avg pages
                </div>
                <div class="text-base font-semibold">
                  {{ formatAverage(scope.avgPageCount) }}
                </div>
              </div>
              <div class="space-y-1">
                <div class="text-muted-foreground text-xs uppercase tracking-[0.18em]">
                  Truncated
                </div>
                <div class="text-base font-semibold">
                  {{ formatCount(scope.truncatedCount) }}
                </div>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>

      <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <Card
          v-for="card in paginationCards"
          :key="card.label"
          class="border-border/60 bg-card/80 backdrop-blur"
        >
          <CardHeader class="pb-2">
            <CardDescription class="text-xs uppercase tracking-[0.18em]">
              {{ card.label }}
            </CardDescription>
            <CardTitle class="text-3xl">
              {{ card.value }}
            </CardTitle>
          </CardHeader>
          <CardContent class="text-muted-foreground text-sm">
            {{ card.detail }}
          </CardContent>
        </Card>
      </div>

      <div class="grid gap-4 xl:grid-cols-2">
        <Card class="overflow-hidden border-border/60">
          <CardHeader>
            <CardTitle>Cache Status Over Time</CardTitle>
            <CardDescription>
              Hit, stale, and miss across the selected window.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <ChartContainer
              :config="cacheChartConfig"
              class="min-h-[320px] w-full"
            >
              <VisXYContainer :data="cacheChartData">
                <VisGroupedBar
                  :x="(d: CacheChartRow) => d.date"
                  :y="[
                    (d: CacheChartRow) => d.hit,
                    (d: CacheChartRow) => d.stale,
                    (d: CacheChartRow) => d.miss,
                  ]"
                  :color="[
                    cacheChartConfig.hit.color,
                    cacheChartConfig.stale.color,
                    cacheChartConfig.miss.color,
                  ]"
                />
                <VisAxis type="x" />
                <VisAxis type="y" />
                <ChartTooltip />
                <ChartCrosshair
                  :template="
                    componentToString(cacheChartConfig, ChartTooltipContent, {
                      labelFormatter: formatBucketLabel,
                    })
                  "
                  :color="[
                    cacheChartConfig.hit.color,
                    cacheChartConfig.stale.color,
                    cacheChartConfig.miss.color,
                  ]"
                />
              </VisXYContainer>
            </ChartContainer>
          </CardContent>
        </Card>

        <Card class="overflow-hidden border-border/60">
          <CardHeader>
            <CardTitle>GitHub Resource Pressure</CardTitle>
            <CardDescription>
              Upstream volume per GitHub resource bucket over the same window.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <ChartContainer
              :config="resourceChartConfig"
              class="min-h-[320px] w-full"
            >
              <VisXYContainer :data="resourceChartData">
                <VisLine
                  v-for="resource in resourceKeys"
                  :key="resource"
                  :x="(d: ResourceChartRow) => d.date as Date"
                  :y="(d: ResourceChartRow) => Number(d[resource] ?? 0)"
                  :color="resourceChartConfig[resource]?.color"
                  curve-type="monotoneX"
                />
                <VisAxis type="x" />
                <VisAxis type="y" />
                <ChartTooltip />
                <ChartCrosshair
                  :template="
                    componentToString(resourceChartConfig, ChartTooltipContent, {
                      labelFormatter: formatBucketLabel,
                    })
                  "
                  :color="resourceKeys.map(resource => resourceChartConfig[resource]?.color)"
                />
              </VisXYContainer>
            </ChartContainer>
          </CardContent>
        </Card>
      </div>

      <div class="grid gap-4 xl:grid-cols-[1.2fr_1fr]">
        <Card class="overflow-hidden border-border/60">
          <CardHeader>
            <CardTitle>Scope Traffic Mix</CardTitle>
            <CardDescription>
              Request volume by cache scope over time. This makes it easier to see whether public promotion is actually absorbing traffic instead of leaving reads in viewer scope.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <ChartContainer
              :config="scopeTrafficChartConfig"
              class="min-h-[320px] w-full"
            >
              <VisXYContainer :data="scopeTrafficChartData">
                <VisLine
                  v-for="scope in scopeKeys"
                  :key="scope"
                  :x="(d: ScopeChartRow) => d.date as Date"
                  :y="(d: ScopeChartRow) => Number(d[scope] ?? 0)"
                  :color="scopeTrafficChartConfig[scope]?.color"
                  curve-type="monotoneX"
                />
                <VisAxis type="x" />
                <VisAxis type="y" />
                <ChartTooltip />
                <ChartCrosshair
                  :template="
                    componentToString(scopeTrafficChartConfig, ChartTooltipContent, {
                      labelFormatter: formatBucketLabel,
                    })
                  "
                  :color="scopeKeys.map(scope => scopeTrafficChartConfig[scope]?.color)"
                />
              </VisXYContainer>
            </ChartContainer>
          </CardContent>
        </Card>

        <Card class="border-border/60">
          <CardHeader>
            <CardTitle>Upstream-Heavy Routes</CardTitle>
            <CardDescription>
              Routes with the highest GitHub call pressure per request. These are the fastest wins for TTL tuning, public promotion, or better revalidation reuse.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Operation</TableHead>
                  <TableHead>Scope</TableHead>
                  <TableHead>Upstream / req</TableHead>
                  <TableHead>Saved</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow
                  v-for="route in upstreamHeavyRoutes"
                  :key="`upstream-heavy:${route.operation}:${route.scope ?? 'unscoped'}`"
                  class="cursor-pointer"
                  :class="{ 'bg-muted/40': isSelectedRoute(route.operation, route.scope) }"
                  @click="selectRouteDrilldown(route.operation, route.scope)"
                >
                  <TableCell class="max-w-[260px]">
                    <div class="flex flex-col gap-1">
                      <span class="font-medium">{{ route.operation }}</span>
                      <span class="text-muted-foreground text-xs">
                        {{ formatDuration(route.avgGithubDurationMs) }} GitHub avg
                      </span>
                    </div>
                  </TableCell>
                  <TableCell>
                    <Badge
                      v-if="route.scope"
                      :variant="scopeVariant(route.scope)"
                    >
                      {{ route.scope }}
                    </Badge>
                    <span v-else class="text-muted-foreground text-xs">—</span>
                  </TableCell>
                  <TableCell>{{ formatPercent(route.upstreamRate) }}</TableCell>
                  <TableCell>
                    <div class="flex flex-wrap items-center gap-2">
                      <span>{{ formatPercent(route.savedRate) }}</span>
                      <Badge
                        v-if="route.truncatedCount > 0"
                        variant="destructive"
                      >
                        truncated
                      </Badge>
                    </div>
                  </TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      </div>

      <Card class="border-border/60">
        <CardHeader class="gap-4 xl:flex-row xl:items-end xl:justify-between">
          <div class="space-y-1">
            <CardTitle>Route Drilldown</CardTitle>
            <CardDescription>
              Inspect one cached operation over time to tune TTL, stale windows, and pagination caps with route-level data.
            </CardDescription>
          </div>

          <div class="flex flex-wrap items-end gap-3">
            <div class="space-y-1">
              <div class="text-muted-foreground text-xs font-medium uppercase tracking-[0.18em]">
                Operation
              </div>
              <Select v-model="selectedOperation">
                <SelectTrigger class="w-[260px]">
                  <SelectValue placeholder="Select operation" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem
                    v-for="operation in drilldownOperationOptions"
                    :key="operation"
                    :value="operation"
                  >
                    {{ operation }}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div class="space-y-1">
              <div class="text-muted-foreground text-xs font-medium uppercase tracking-[0.18em]">
                Scope
              </div>
              <Select v-model="selectedScope">
                <SelectTrigger class="w-[180px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">
                    All scopes
                  </SelectItem>
                  <SelectItem
                    v-for="scope in drilldownScopeOptions"
                    :key="scope"
                    :value="scope"
                  >
                    {{ formatScopeLabel(scope) }}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
        </CardHeader>
        <CardContent class="space-y-4">
          <div
            v-if="drilldownErrorMessage"
            class="text-destructive text-sm"
          >
            {{ drilldownErrorMessage }}
          </div>

          <div
            v-if="isDrilldownLoading && !drilldown"
            class="grid gap-4 md:grid-cols-2 2xl:grid-cols-4"
          >
            <Skeleton
              v-for="index in 4"
              :key="`drilldown-skeleton-${index}`"
              class="h-28"
            />
          </div>

          <template v-else-if="drilldown?.summary">
            <div class="grid gap-4 md:grid-cols-2 2xl:grid-cols-4">
              <Card
                v-for="card in drilldownSummaryCards"
                :key="card.label"
                class="border-border/60 bg-card/60"
              >
                <CardHeader class="pb-2">
                  <CardDescription class="text-xs uppercase tracking-[0.18em]">
                    {{ card.label }}
                  </CardDescription>
                  <CardTitle class="text-2xl">
                    {{ card.value }}
                  </CardTitle>
                </CardHeader>
                <CardContent class="text-muted-foreground text-sm">
                  {{ card.detail }}
                </CardContent>
              </Card>
            </div>

            <div class="grid gap-4 xl:grid-cols-[1.4fr_1fr]">
              <Card class="overflow-hidden border-border/60">
                <CardHeader>
                  <div class="flex items-center justify-between gap-3">
                    <div>
                      <CardTitle>{{ drilldown.summary.operation }}</CardTitle>
                      <CardDescription>
                        {{ selectedScope === 'all' ? 'All scopes' : formatScopeLabel(selectedScope) }}
                      </CardDescription>
                    </div>
                    <Badge
                      :variant="selectedScope === 'all' ? 'outline' : scopeVariant(selectedScope)"
                    >
                      {{ selectedScope === 'all' ? 'all' : selectedScope }}
                    </Badge>
                  </div>
                </CardHeader>
                <CardContent>
                  <ChartContainer
                    :config="drilldownChartConfig"
                    class="min-h-[320px] w-full"
                  >
                    <VisXYContainer :data="drilldownChartData">
                      <VisGroupedBar
                        :x="(d: DrilldownChartRow) => d.date"
                        :y="[
                          (d: DrilldownChartRow) => d.hit,
                          (d: DrilldownChartRow) => d.stale,
                          (d: DrilldownChartRow) => d.miss,
                        ]"
                        :color="[
                          drilldownChartConfig.hit.color,
                          drilldownChartConfig.stale.color,
                          drilldownChartConfig.miss.color,
                        ]"
                      />
                      <VisLine
                        :x="(d: DrilldownChartRow) => d.date"
                        :y="(d: DrilldownChartRow) => d.upstreamCalls"
                        :color="drilldownChartConfig.upstreamCalls.color"
                        curve-type="monotoneX"
                      />
                      <VisAxis type="x" />
                      <VisAxis type="y" />
                      <ChartTooltip />
                      <ChartCrosshair
                        :template="
                          componentToString(drilldownChartConfig, ChartTooltipContent, {
                            labelFormatter: formatBucketLabel,
                          })
                        "
                        :color="[
                          drilldownChartConfig.hit.color,
                          drilldownChartConfig.stale.color,
                          drilldownChartConfig.miss.color,
                          drilldownChartConfig.upstreamCalls.color,
                        ]"
                      />
                    </VisXYContainer>
                  </ChartContainer>
                </CardContent>
              </Card>

              <Card class="border-border/60">
                <CardHeader>
                  <CardTitle>Bucket Details</CardTitle>
                  <CardDescription>
                    Per-bucket activity for the selected route. This is the quickest way to see whether misses or rebuilds cluster in specific periods.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Bucket</TableHead>
                        <TableHead>Req</TableHead>
                        <TableHead>Upstream</TableHead>
                        <TableHead>Pages</TableHead>
                        <TableHead>Trunc.</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      <TableRow
                        v-for="point in drilldown.series"
                        :key="point.bucketStart"
                      >
                        <TableCell>{{ formatBucketLabel(point.bucketStart) }}</TableCell>
                        <TableCell>{{ formatCount(point.requests) }}</TableCell>
                        <TableCell>{{ formatCount(point.upstreamCalls) }}</TableCell>
                        <TableCell>{{ formatAverage(point.avgPageCount) }}</TableCell>
                        <TableCell>
                          <Badge :variant="point.truncatedCount > 0 ? 'destructive' : 'outline'">
                            {{ formatCount(point.truncatedCount) }}
                          </Badge>
                        </TableCell>
                      </TableRow>
                    </TableBody>
                  </Table>
                </CardContent>
              </Card>
            </div>
          </template>

          <div
            v-else
            class="text-muted-foreground py-6 text-sm"
          >
            No drilldown data is available for the selected route in this window.
          </div>
        </CardContent>
      </Card>

      <div class="grid gap-4 xl:grid-cols-[1.4fr_1fr]">
        <Card class="border-border/60">
          <CardHeader>
            <CardTitle>Heavy Paginated Aggregates</CardTitle>
            <CardDescription>
              Top tracked routes building multi-page collections in the current window. Use this table to spot the operations that need tighter TTLs, better caps, or product-level pagination.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Table v-if="paginatedRoutes.length > 0">
              <TableHeader>
                <TableRow>
                  <TableHead>Operation</TableHead>
                  <TableHead>Scope</TableHead>
                  <TableHead>Loads</TableHead>
                  <TableHead>Avg pages</TableHead>
                  <TableHead>Avg items</TableHead>
                  <TableHead>Truncated</TableHead>
                  <TableHead>Rebuild avg</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow
                  v-for="route in paginatedRoutes"
                  :key="`paginated:${route.operation}:${route.scope ?? 'unscoped'}`"
                  class="cursor-pointer"
                  :class="{ 'bg-muted/40': isSelectedRoute(route.operation, route.scope) }"
                  @click="selectRouteDrilldown(route.operation, route.scope)"
                >
                  <TableCell class="max-w-[320px]">
                    <div class="flex flex-col gap-1">
                      <span class="font-medium">{{ route.operation }}</span>
                      <span class="text-muted-foreground text-xs">
                        last seen {{ formatDateTime(route.lastSeenAt) }}
                      </span>
                    </div>
                  </TableCell>
                  <TableCell>
                    <Badge
                      v-if="route.scope"
                      :variant="scopeVariant(route.scope)"
                    >
                      {{ route.scope }}
                    </Badge>
                    <span v-else class="text-muted-foreground text-xs">—</span>
                  </TableCell>
                  <TableCell>{{ formatCount(route.paginatedLoads) }}</TableCell>
                  <TableCell>{{ formatAverage(route.avgPageCount) }}</TableCell>
                  <TableCell>{{ formatAverage(route.avgItemCount) }}</TableCell>
                  <TableCell>
                    <Badge :variant="route.truncatedCount > 0 ? 'destructive' : 'outline'">
                      {{ formatCount(route.truncatedCount) }}
                    </Badge>
                  </TableCell>
                  <TableCell>{{ formatDuration(route.avgPaginationDurationMs) }}</TableCell>
                </TableRow>
              </TableBody>
            </Table>
            <div
              v-else
              class="text-muted-foreground py-8 text-sm"
            >
              No paginated collection rebuilds were recorded in this window.
            </div>
          </CardContent>
        </Card>

        <Card class="border-border/60">
          <CardHeader>
            <CardTitle>Truncated Collections</CardTitle>
            <CardDescription>
              Top tracked aggregates that hit the current safety cap. These are the first candidates for product-level pagination or route-specific caps.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Table v-if="truncatedRoutes.length > 0">
              <TableHeader>
                <TableRow>
                  <TableHead>Operation</TableHead>
                  <TableHead>Scope</TableHead>
                  <TableHead>Hits</TableHead>
                  <TableHead>Avg items</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow
                  v-for="route in truncatedRoutes"
                  :key="`truncated:${route.operation}:${route.scope ?? 'unscoped'}`"
                  class="cursor-pointer"
                  :class="{ 'bg-muted/40': isSelectedRoute(route.operation, route.scope) }"
                  @click="selectRouteDrilldown(route.operation, route.scope)"
                >
                  <TableCell class="max-w-[260px]">
                    <div class="flex flex-col gap-1">
                      <span class="font-medium">{{ route.operation }}</span>
                      <span class="text-muted-foreground text-xs">
                        {{ formatDuration(route.avgPaginationDurationMs) }} average rebuild
                      </span>
                    </div>
                  </TableCell>
                  <TableCell>
                    <Badge
                      v-if="route.scope"
                      :variant="scopeVariant(route.scope)"
                    >
                      {{ route.scope }}
                    </Badge>
                    <span v-else class="text-muted-foreground text-xs">—</span>
                  </TableCell>
                  <TableCell>
                    <Badge variant="destructive">
                      {{ formatCount(route.truncatedCount) }}
                    </Badge>
                  </TableCell>
                  <TableCell>{{ formatAverage(route.avgItemCount) }}</TableCell>
                </TableRow>
              </TableBody>
            </Table>
            <div
              v-else
              class="text-muted-foreground py-8 text-sm"
            >
              No collection hit the configured pagination cap in this window.
            </div>
          </CardContent>
        </Card>
      </div>

      <div class="grid gap-4 xl:grid-cols-[1.6fr_1fr]">
        <Card class="border-border/60">
          <CardHeader>
            <CardTitle>Routes Needing Tuning</CardTitle>
            <CardDescription>
              Operations split by cache scope, so the same route can appear once for viewer traffic and once for public traffic.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Operation</TableHead>
                  <TableHead>Scope</TableHead>
                  <TableHead>Requests</TableHead>
                  <TableHead>Hit</TableHead>
                  <TableHead>Upstream</TableHead>
                  <TableHead>304</TableHead>
                  <TableHead>TTL</TableHead>
                  <TableHead>GitHub avg</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow
                  v-for="route in overview.routes"
                  :key="`${route.operation}:${route.scope ?? 'unscoped'}`"
                  class="cursor-pointer"
                  :class="{ 'bg-muted/40': isSelectedRoute(route.operation, route.scope) }"
                  @click="selectRouteDrilldown(route.operation, route.scope)"
                >
                  <TableCell class="max-w-[320px]">
                    <div class="flex flex-col gap-1">
                      <span class="font-medium">{{ route.operation }}</span>
                      <div class="flex flex-wrap items-center gap-2 text-xs">
                        <span class="text-muted-foreground">
                          stale window {{ formatDuration(route.staleMs) }}
                        </span>
                        <Badge
                          v-if="route.paginatedLoads > 0"
                          variant="outline"
                        >
                          {{ formatCount(route.paginatedLoads) }} paginated
                        </Badge>
                        <Badge
                          v-if="route.truncatedCount > 0"
                          variant="destructive"
                        >
                          {{ formatCount(route.truncatedCount) }} truncated
                        </Badge>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <Badge
                      v-if="route.scope"
                      :variant="scopeVariant(route.scope)"
                    >
                      {{ route.scope }}
                    </Badge>
                    <span v-else class="text-muted-foreground text-xs">—</span>
                  </TableCell>
                  <TableCell>{{ formatCount(route.requests) }}</TableCell>
                  <TableCell>{{ formatPercent(route.hitRate) }}</TableCell>
                  <TableCell>{{ formatCount(route.upstreamCalls) }}</TableCell>
                  <TableCell>{{ formatPercent(route.notModifiedRate) }}</TableCell>
                  <TableCell>{{ formatDuration(route.ttlMs) }}</TableCell>
                  <TableCell>{{ formatDuration(route.avgGithubDurationMs) }}</TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </CardContent>
        </Card>

        <Card class="border-border/60">
          <CardHeader>
            <CardTitle>Users Under Pressure</CardTitle>
            <CardDescription>
              Users getting closest to the rate limit during the selected window.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>User</TableHead>
                  <TableHead>Lowest</TableHead>
                  <TableHead>Upstream</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow
                  v-for="entry in overview.users"
                  :key="entry.userId"
                >
                  <TableCell>
                    <div class="flex flex-col gap-1">
                      <span class="font-medium">
                        {{ entry.user?.name ?? entry.userId }}
                      </span>
                      <span class="text-muted-foreground text-xs">
                        {{ entry.user?.email ?? entry.lastOperation ?? '—' }}
                      </span>
                    </div>
                  </TableCell>
                  <TableCell>
                    <Badge :variant="pressureVariant(entry.lowestRemainingPct)">
                      {{ formatPercent(entry.lowestRemainingPct) }}
                    </Badge>
                  </TableCell>
                  <TableCell>{{ formatCount(entry.upstreamCalls) }}</TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      </div>

      <Card class="border-border/60">
        <CardHeader>
          <CardTitle>Current Rate Limits</CardTitle>
          <CardDescription>
            Snapshot of the latest GitHub rate-limit headers observed per resource and user.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>User</TableHead>
                <TableHead>Resource</TableHead>
                <TableHead>Remaining</TableHead>
                <TableHead>Reset</TableHead>
                <TableHead>Last operation</TableHead>
                <TableHead>Updated</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="item in overview.currentRateLimits"
                :key="`${item.userId}:${item.resource}`"
              >
                <TableCell>
                  <div class="flex flex-col gap-1">
                    <span class="font-medium">
                      {{ item.user?.name ?? item.userId }}
                    </span>
                    <span class="text-muted-foreground text-xs">
                      {{ item.user?.email ?? '—' }}
                    </span>
                  </div>
                </TableCell>
                <TableCell>
                  <Badge variant="outline">
                    {{ item.resource }}
                  </Badge>
                </TableCell>
                <TableCell>
                  <div class="flex flex-col gap-1">
                    <span>{{ formatRateLimitRemaining(item.remaining, item.limit) }}</span>
                    <Badge :variant="pressureVariant(item.remainingPct)">
                      {{ formatPercent(item.remainingPct) }}
                    </Badge>
                  </div>
                </TableCell>
                <TableCell>{{ formatReset(item.reset) }}</TableCell>
                <TableCell class="max-w-[320px] truncate">
                  {{ item.lastOperation ?? item.lastRoute ?? '—' }}
                </TableCell>
                <TableCell>{{ formatDateTime(item.updatedAt) }}</TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </template>
  </div>
</template>
