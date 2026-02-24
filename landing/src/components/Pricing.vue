<script setup lang="ts">
import { CheckIcon, SparklesIcon } from 'lucide-vue-next'

const tiers = [
  {
    name: 'Free',
    id: 'free',
    href: '#download',
    price: '$0',
    description: 'Full local Git client for everyday development and review workflows.',
    features: [
      'Local repository management',
      'Branch switching, fetch and push',
      'Stage/unstage by file or hunk',
      'Commit, amend, and undo flows',
      'Rebase, stash, cherry-pick, conflict helpers',
      'Keyboard-first command palette',
    ],
    featured: false,
    cta: 'Download app',
  },
  {
    name: 'Pro',
    id: 'pro',
    href: '#download',
    price: { monthly: '$19', annually: '$199' },
    description: 'Unlock GitHub workflows directly inside Reviu.',
    features: [
      'GitHub notifications inbox',
      'Repository tabs: Overview, Readme, Code, Pull Requests, Issues',
      'Branch-aware Readme and Code browsing',
      'PR review with inline/split diff',
      'Create, edit, reply, and delete review comments',
      'Issue and PR context without leaving the app',
    ],
    featured: true,
    cta: 'Get Reviu Pro',
  },
]
</script>

<template>
  <section id="pricing" class="group/tiers py-24 sm:py-32">
    <div class="mx-auto max-w-7xl px-6 lg:px-8">
      <div class="mx-auto max-w-4xl text-center">
        <h2 class="text-base/7 font-semibold text-primary">Pricing</h2>
        <p class="mt-2 text-5xl font-semibold tracking-tight text-balance text-gray-900 sm:text-6xl dark:text-white">Free local Git.<br /> Pro GitHub workflows.</p>
      </div>
      <p class="mx-auto mt-6 max-w-2xl text-center text-lg font-medium text-pretty text-gray-600 sm:text-xl/8 dark:text-gray-400">Start with the free desktop client and upgrade when you need integrated GitHub review operations.</p>
      <div class="mt-16 flex justify-center">
        <fieldset aria-label="Payment frequency">
          <div class="grid grid-cols-2 gap-x-1 rounded-full p-1 text-center text-xs/5 font-semibold inset-ring inset-ring-gray-200 dark:inset-ring-white/10">
            <label class="group relative rounded-full px-2.5 py-1 has-checked:bg-primary">
              <input type="radio" name="frequency" value="monthly" checked class="absolute inset-0 appearance-none rounded-full" />
              <span class="text-muted-foreground group-has-checked:text-white">Monthly</span>
            </label>
            <label class="group relative rounded-full px-2.5 py-1 has-checked:bg-primary">
              <input type="radio" name="frequency" value="annually" class="absolute inset-0 appearance-none rounded-full" />
              <span class="text-muted-foreground group-has-checked:text-white">Annually</span>
            </label>
          </div>
        </fieldset>
      </div>
      <div class="isolate mx-auto lg:px-20 xl:px-40 mt-10 grid max-w-md grid-cols-1 gap-8 lg:mx-0 lg:max-w-none lg:grid-cols-2">
        <div v-for="tier in tiers" :key="tier.id" class="group/tier rounded-3xl p-8 ring-1 xl:p-10 bg-background ring-muted dark:data-featured:ring-2 data-featured:ring-primary" :data-featured="tier.featured ? 'true' : undefined">
          <div class="flex items-center gap-2">
            <h3 :id="`tier-${tier.id}`" class="text-lg/8 font-semibold text-foreground group-data-featured/tier:text-primary">{{ tier.name }}</h3>
            <SparklesIcon v-if="tier.featured" class="size-4 text-primary" aria-hidden="true" />
          </div>
          <p class="mt-4 text-sm/6 text-muted-foreground ">{{ tier.description }}</p>
          <p v-if="typeof tier.price === 'string'" class="mt-6 text-4xl font-semibold tracking-tight text-foreground">{{ tier.price }}</p>
          <template v-else>
            <p class="mt-6 flex items-baseline gap-x-1 group-not-has-[[name=frequency][value=monthly]:checked]/tiers:hidden">
              <span class="text-4xl font-semibold tracking-tight text-foreground">{{ tier.price.monthly }}</span>
              <span class="text-sm/6 font-semibold text-muted-foreground">/month</span>
            </p>
            <p class="mt-6 flex items-baseline gap-x-1 group-not-has-[[name=frequency][value=annually]:checked]/tiers:hidden">
              <span class="text-4xl font-semibold tracking-tight text-foreground">{{ tier.price.annually }}</span>
              <span class="text-sm/6 font-semibold text-muted-foreground">/year</span>
            </p>
          </template>
          <a :href="tier.href" :aria-describedby="`tier-${tier.id}`" class="mt-6 block w-full rounded-md bg-primary px-3 py-2 text-center text-sm/6 font-semibold text-white shadow-xs hover:bg-primary/80 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary group-data-featured/tier:focus-visible:outline-white/75 dark:shadow-none">{{ tier.cta }}</a>
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
