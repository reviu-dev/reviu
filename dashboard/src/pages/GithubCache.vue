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
type GithubCacheOverview = Awaited<ReturnType<AdminOverviewResponse['json']>>
type CacheChartRow = GithubCacheOverview['cacheStatusSeries'][number] & { date: Date }
type ResourceChartRow = { date: Date } & Record<string, number | Date>

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

let requestId = 0

const compactNumberFormatter = new Intl.NumberFormat('en-US', {
  notation: 'compact',
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

async function fetchOverview() {
  const currentRequestId = ++requestId

  isLoading.value = true
  errorMessage.value = null

  try {
    const response = await adminClient['github-cache'].overview.$get({
      query: {
        windowMinutes: String(Number(selectedWindowMinutes.value)),
        limit: '12',
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

watch(selectedWindowMinutes, () => {
  fetchOverview()
}, { immediate: true })

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

function formatCount(value: number | null | undefined) {
  if (value == null) {
    return '—'
  }

  return compactNumberFormatter.format(value)
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

      <div class="flex flex-wrap items-center gap-3">
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
      <div class="grid gap-4 md:grid-cols-2 2xl:grid-cols-4">
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

      <div class="grid gap-4 xl:grid-cols-[1.6fr_1fr]">
        <Card class="border-border/60">
          <CardHeader>
            <CardTitle>Routes Needing Tuning</CardTitle>
            <CardDescription>
              Operations that consume the most GitHub upstream calls or still keep a high miss rate.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Operation</TableHead>
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
                  :key="route.operation"
                >
                  <TableCell class="max-w-[320px]">
                    <div class="flex flex-col gap-1">
                      <span class="font-medium">{{ route.operation }}</span>
                      <div class="flex flex-wrap gap-2">
                        <Badge
                          v-if="route.scope"
                          variant="outline"
                        >
                          {{ route.scope }}
                        </Badge>
                        <span class="text-muted-foreground text-xs">
                          stale window {{ formatDuration(route.staleMs) }}
                        </span>
                      </div>
                    </div>
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
