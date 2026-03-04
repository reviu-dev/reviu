<script setup lang="ts">
import { CheckIcon, SparklesIcon } from 'lucide-vue-next'
import { useIntersectionObserver } from '../composables/useIntersectionObserver'

const { isVisible } = useIntersectionObserver(0.3)

const tiers = [
  {
    name: 'Free',
    id: 'free',
    href: '#download',
    available: true,
    price: '$0',
    description: 'Full local Git client for everyday development and review workflows.',
    features: [
      'Keyboard-first command palette and shortcuts',
      'Common Git operations: repository management, branch switching, fetch, stash...',
      'Advanced Git operations: interactive rebase, cherry-pick, conflicts...',
      'Local diff workflow with inline/split views',
      'Manage your Git config'
    ],
    featured: false,
    cta: 'Download app',
  },
  {
    name: 'Pro',
    id: 'pro',
    href: '#download',
    available: false,
    price: '$19/month',
    description: 'Unlock GitHub workflows directly inside Reviu.',
    features: [
      'Keyboard-first command palette for GitHub actions',
      'GitHub notifications feed',
      'Browse GitHub repositories with Code, Pull Requests, Issues...',
      'PR review with inline and split diff modes',
      'Browser extension to open GitHub URLs in Reviu',
    ],
    featured: true,
    cta: 'Coming soon',
  },
]
</script>

<template>
  <section ref="targetRef" id="pricing" class="group/tiers py-24 sm:py-32">
    <div class="mx-auto max-w-7xl px-6 lg:px-8">
      <div 
        class="mx-auto max-w-4xl text-center transition-all duration-700 ease-out transform"
        :class="[isVisible ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-8']"
      >
        <h2 class="text-base/7 font-semibold text-primary">Pricing</h2>
        <p class="mt-2 text-5xl font-semibold tracking-tight text-balance text-foreground sm:text-6xl">Simple pricing for local Git and GitHub workflows.</p>
        <p class="mx-auto mt-6 max-w-2xl text-center text-lg font-medium text-pretty text-gray-600 sm:text-xl/8 dark:text-gray-400">Use Free for full local Git workflows, then upgrade to Pro at $19/month when you need GitHub notifications, repositories, PR reviews, and issues.</p>
      </div>
      <div class="isolate mx-auto lg:px-20 xl:px-40 mt-10 grid max-w-md grid-cols-1 gap-8 lg:mx-0 lg:max-w-none lg:grid-cols-2">
        <div 
          v-for="(tier, index) in tiers" 
          :key="tier.id" 
          class="group/tier rounded-3xl p-8 ring-1 xl:p-10 bg-background ring-muted dark:data-featured:ring-2 data-featured:ring-primary transition-all duration-700 ease-out transform"
          :class="[isVisible ? 'opacity-100 translate-y-0' : 'opacity-0 translate-y-8']"
          :style="{ transitionDelay: `${index * 150 + 200}ms` }"
          :data-featured="tier.featured ? 'true' : undefined"
        >
          <div class="flex items-center gap-2">
            <h3 :id="`tier-${tier.id}`" class="text-lg/8 font-semibold text-foreground group-data-featured/tier:text-primary">{{ tier.name }}</h3>
            <SparklesIcon v-if="tier.featured" class="size-4 text-primary" aria-hidden="true" />
          </div>
          <p class="mt-4 text-sm/6 text-muted-foreground ">{{ tier.description }}</p>
          <p class="mt-6 text-4xl font-semibold tracking-tight text-foreground">{{ tier.price }}</p>
          <template v-if="tier.id === 'pro'">
            <p class="mt-2 text-sm/6 text-muted-foreground">14-day free trial, no payment method required.</p>
          </template>
          <a
            v-if="tier.available"
            :href="tier.href"
            :aria-describedby="`tier-${tier.id}`"
            class="mt-6 block hover:scale-105 transition-transform w-full rounded-md bg-primary px-3 py-2 text-center text-sm/6 font-semibold text-white shadow-xs hover:bg-primary/80 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary group-data-featured/tier:focus-visible:outline-white/75 dark:shadow-none"
          >
            {{ tier.cta }}
          </a>
          <div v-else class="mt-6">
            <button
              type="button"
              disabled
              class="block w-full cursor-not-allowed rounded-md bg-muted px-3 py-2 text-center text-sm/6 font-semibold text-muted-foreground"
            >
              {{ tier.cta }}
            </button>
          </div>
          <ul role="list" class="mt-8 space-y-3 text-sm/6 text-muted-foreground xl:mt-10">
            <li v-for="feature in tier.features" :key="feature" class="flex gap-x-3">
              <CheckIcon class="h-6 w-5 flex-none text-primary" aria-hidden="true" />
              {{ feature }}
            </li>
          </ul>
        </div>
      </div>
      <p class="mx-auto mt-8 max-w-2xl text-center text-sm text-muted-foreground">Start free, upgrade to Pro when you need GitHub integration.</p>
    </div>
  </section>
</template>
