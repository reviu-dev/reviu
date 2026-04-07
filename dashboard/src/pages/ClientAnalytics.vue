<script setup lang="ts">
import type { ChartConfig } from '@/components/ui/chart'
import { VisAxis, VisGroupedBar, VisXYContainer } from '@unovis/vue'
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

type ClientAnalyticsResponse = Awaited<ReturnType<(typeof adminClient)['client-analytics']['$get']>>
type ClientAnalyticsData = Awaited<ReturnType<ClientAnalyticsResponse['json']>>

const WINDOW_OPTIONS = [
  { label: '1 hr', value: '60' },
  { label: '24 hr', value: '1440' },
  { label: '7 days', value: '10080' },
]

const LEGACY_ROUTES = new Set([
  '/github/pr/latest',
  '/github/pr/need-reviews',
])

const versionChartConfig = {
  requests: {
    label: 'Requests',
    color: 'var(--chart-1)',
  },
} satisfies ChartConfig

const selectedWindowMinutes = ref('1440')
const data = ref<ClientAnalyticsData | null>(null)
const isLoading = ref(false)
const errorMessage = ref<string | null>(null)
const lastFetchedAt = ref<number | null>(null)

let requestId = 0

const compactNumberFormatter = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 1,
})

const dateTimeFormatter = new Intl.DateTimeFormat('en-US', {
  day: '2-digit',
  month: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
})

function formatCount(value: number | null | undefined) {
  if (value == null)
    return '-'
  return compactNumberFormatter.format(value)
}

function formatDateTime(ts: number) {
  return dateTimeFormatter.format(new Date(ts))
}

async function fetchData() {
  const currentRequestId = ++requestId

  isLoading.value = true
  errorMessage.value = null

  try {
    const response = await adminClient['client-analytics'].$get({
      query: {
        windowMinutes: String(Number(selectedWindowMinutes.value)),
      },
    })

    if (!response.ok) {
      const payload = await response.json().catch(() => null) as { error?: string, message?: string } | null
      throw new Error(payload?.error ?? payload?.message ?? 'Failed to load client analytics')
    }

    const result = await response.json()
    if (currentRequestId !== requestId)
      return

    data.value = result
    lastFetchedAt.value = Date.now()
  }
  catch (error) {
    if (currentRequestId === requestId) {
      errorMessage.value = error instanceof Error ? error.message : 'Unknown error'
    }
  }
  finally {
    if (currentRequestId === requestId) {
      isLoading.value = false
    }
  }
}

watch(selectedWindowMinutes, () => fetchData(), { immediate: true })

const summaryCards = computed(() => {
  if (!data.value)
    return []
  const { versions, routes } = data.value
  const totalRequests = versions.reduce((sum, v) => sum + v.requests, 0)
  const uniqueVersions = new Set(versions.map(v => v.clientVersion)).size
  const totalUniqueUsers = new Set(versions.flatMap((v) => {
    // approximate: sum unique users per version (may overcount across versions)
    return Array.from({ length: v.uniqueUsers }, (_, i) => `${v.clientVersion}-${i}`)
  })).size
  const legacyRequests = routes
    .filter(r => LEGACY_ROUTES.has(r.route))
    .reduce((sum, r) => sum + r.requests, 0)

  return [
    { label: 'Total Requests', value: formatCount(totalRequests) },
    { label: 'Client Versions', value: String(uniqueVersions) },
    { label: 'Unique Users', value: formatCount(totalUniqueUsers) },
    { label: 'Legacy Route Hits', value: formatCount(legacyRequests), highlight: legacyRequests > 0 },
  ]
})

interface VersionChartRow {
  version: string
  requests: number
}

const versionChartData = computed<VersionChartRow[]>(() => {
  if (!data.value)
    return []

  const grouped = new Map<string, number>()
  for (const v of data.value.versions) {
    grouped.set(v.clientVersion, (grouped.get(v.clientVersion) ?? 0) + v.requests)
  }

  return Array.from(grouped.entries(), ([version, requests]) => ({ version, requests }))
    .sort((a, b) => a.version.localeCompare(b.version, undefined, { numeric: true }))
})

function isLegacyRoute(route: string) {
  return LEGACY_ROUTES.has(route)
}
</script>

<template>
  <div class="container mx-auto space-y-6">
    <div class="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
      <div class="space-y-1">
        <h1 class="text-3xl font-bold tracking-tight">
          Client Analytics
        </h1>
        <p class="text-muted-foreground max-w-3xl">
          Track active desktop client versions and route usage to identify when legacy endpoints can be safely removed.
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
          @click="fetchData"
        >
          <RefreshCw class="size-4" :class="{ 'animate-spin': isLoading }" />
          Refresh
        </Button>
      </div>
    </div>

    <div class="flex flex-wrap items-center gap-3 text-sm">
      <Badge v-if="lastFetchedAt" variant="outline">
        Updated {{ formatDateTime(lastFetchedAt) }}
      </Badge>
      <Badge v-if="errorMessage" variant="destructive">
        {{ errorMessage }}
      </Badge>
    </div>

    <div
      v-if="!data && isLoading"
      class="grid gap-4 md:grid-cols-2 lg:grid-cols-4"
    >
      <Skeleton
        v-for="index in 4"
        :key="index"
        class="h-32"
      />
    </div>

    <template v-if="data">
      <!-- Summary Cards -->
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
            <CardTitle class="text-3xl" :class="{ 'text-destructive': card.highlight }">
              {{ card.value }}
            </CardTitle>
          </CardHeader>
        </Card>
      </div>

      <!-- Version Distribution Chart -->
      <Card v-if="versionChartData.length > 0">
        <CardHeader>
          <CardTitle>Requests by Version</CardTitle>
          <CardDescription>Request count per client version</CardDescription>
        </CardHeader>
        <CardContent>
          <ChartContainer :config="versionChartConfig" class="min-h-[260px] w-full">
            <VisXYContainer :data="versionChartData">
              <VisGroupedBar
                :x="(_: VersionChartRow, i: number) => i"
                :y="[(d: VersionChartRow) => d.requests]"
                :color="['var(--chart-1)']"
                :bar-padding="0.3"
                :rounded-corners="4"
              />
              <VisAxis
                type="x"
                :tick-format="(_: number, i: number) => versionChartData[i]?.version ?? ''"
              />
              <VisAxis type="y" />
              <ChartTooltip />
              <ChartCrosshair
                :template="
                  componentToString(versionChartConfig, ChartTooltipContent, {})
                "
                :color="[versionChartConfig.requests.color]"
              />
            </VisXYContainer>
          </ChartContainer>
        </CardContent>
      </Card>

      <!-- Version Distribution Table -->
      <Card>
        <CardHeader>
          <CardTitle>Version Distribution</CardTitle>
          <CardDescription>Active client versions with platform details</CardDescription>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Version</TableHead>
                <TableHead>Platform</TableHead>
                <TableHead>Arch</TableHead>
                <TableHead class="text-right">
                  Requests
                </TableHead>
                <TableHead class="text-right">
                  Unique Users
                </TableHead>
                <TableHead class="text-right">
                  Last Seen
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="version in data.versions" :key="`${version.clientVersion}-${version.clientPlatform}-${version.clientArch}`">
                <TableCell class="font-mono font-medium">
                  {{ version.clientVersion }}
                </TableCell>
                <TableCell>{{ version.clientPlatform ?? '-' }}</TableCell>
                <TableCell>{{ version.clientArch ?? '-' }}</TableCell>
                <TableCell class="text-right">
                  {{ formatCount(version.requests) }}
                </TableCell>
                <TableCell class="text-right">
                  {{ version.uniqueUsers }}
                </TableCell>
                <TableCell class="text-right text-muted-foreground">
                  {{ formatDateTime(version.lastSeenAt) }}
                </TableCell>
              </TableRow>
              <TableRow v-if="data.versions.length === 0">
                <TableCell colspan="6" class="text-center text-muted-foreground py-8">
                  No data for this time window
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <!-- Route Usage by Version -->
      <Card>
        <CardHeader>
          <CardTitle>Route Usage by Version</CardTitle>
          <CardDescription>API routes called by each client version — legacy routes are highlighted</CardDescription>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Method</TableHead>
                <TableHead>Route</TableHead>
                <TableHead>Client Version</TableHead>
                <TableHead class="text-right">
                  Requests
                </TableHead>
                <TableHead class="text-right">
                  Last Seen
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="(route, index) in data.routes"
                :key="index"
                :class="{ 'bg-destructive/5': isLegacyRoute(route.route) }"
              >
                <TableCell>
                  <Badge variant="secondary" class="font-mono text-xs">
                    {{ route.method }}
                  </Badge>
                </TableCell>
                <TableCell class="font-mono">
                  {{ route.route }}
                  <Badge v-if="isLegacyRoute(route.route)" variant="destructive" class="ml-2 text-xs">
                    legacy
                  </Badge>
                </TableCell>
                <TableCell class="font-mono">
                  {{ route.clientVersion }}
                </TableCell>
                <TableCell class="text-right">
                  {{ formatCount(route.requests) }}
                </TableCell>
                <TableCell class="text-right text-muted-foreground">
                  {{ formatDateTime(route.lastSeenAt) }}
                </TableCell>
              </TableRow>
              <TableRow v-if="data.routes.length === 0">
                <TableCell colspan="5" class="text-center text-muted-foreground py-8">
                  No data for this time window
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </template>
  </div>
</template>
