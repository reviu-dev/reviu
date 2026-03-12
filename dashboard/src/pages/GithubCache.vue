<script setup lang="ts">
import type { ChartConfig } from '@/components/ui/chart'
import { VisGroupedBar, VisXYContainer } from '@unovis/vue'
import {
  ChartContainer,
  ChartCrosshair,
  ChartTooltip,
  ChartTooltipContent,
  componentToString,
} from '@/components/ui/chart'

const chartData = [
  { date: new Date('2024-01-01'), desktop: 186, mobile: 80 },
  { date: new Date('2024-02-01'), desktop: 305, mobile: 200 },
  { date: new Date('2024-03-01'), desktop: 237, mobile: 120 },
]
type Data = (typeof chartData)[number]

const chartConfig = {
  desktop: {
    label: 'Desktop',
    color: 'var(--chart-1)',
  },
  mobile: {
    label: 'Mobile',
    color: 'var(--chart-2)',
  },
} satisfies ChartConfig
</script>

<template>
  <ChartContainer :config="chartConfig" class="min-h-[400px] w-full">
    <VisXYContainer :data="chartData">
      <VisGroupedBar
        :x="(d: Data) => d.date"
        :y="[(d: Data) => d.desktop, (d: Data) => d.mobile]"
        :color="[chartConfig.desktop.color, chartConfig.mobile.color]"
      />
      <ChartTooltip />
      <ChartCrosshair
        :template="
          componentToString(chartConfig, ChartTooltipContent, {
            labelFormatter(d) {
              return new Date(d).toLocaleDateString('en-US', {
                month: 'long',
              });
            },
          })
        "
        :color="[chartConfig.desktop.color, chartConfig.mobile.color]"
      />
    </VisXYContainer>
  </ChartContainer>
</template>
