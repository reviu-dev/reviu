
<script setup lang="ts">
import {
  BellIcon,
  CommandIcon,
  FolderIcon,
  GitCommitHorizontalIcon,
  GitPullRequestIcon,
  SplitIcon,
} from 'lucide-vue-next'

type Feature = {
  name: string
  description: string
  icon: unknown
  plan: 'Free' | 'Pro'
}

const features: Feature[] = [
  {
    name: 'Keyboard-first command palette',
    description: 'Run commit, fetch, push, rebase, stash, and cherry-pick actions with shortcuts instead of click-heavy menus.',
    icon: CommandIcon,
    plan: 'Free',
  },
  {
    name: 'Advanced local Git operations',
    description: 'Handle common Git operations in one flow: branch switching, fetch/push/force push, stage or unstage, commit and undo, plus interactive rebase, stash, and cherry-pick.',
    icon: GitCommitHorizontalIcon,
    plan: 'Free',
  },
  {
    name: 'Local diff and conflict resolution',
    description: 'Review local changes with inline and split diff views, stage by hunk, and resolve merge conflicts without leaving the desktop app.',
    icon: SplitIcon,
    plan: 'Free',
  },
  {
    name: 'GitHub notifications in-app',
    description: 'Track the work that needs attention from a desktop notifications feed without context switching.',
    icon: BellIcon,
    plan: 'Pro',
  },
  {
    name: 'Browse GitHub repos, PRs, and issues',
    description: 'Open repository context from one place with Overview, Readme, Code, Pull Requests, and Issues directly in Reviu.',
    icon: FolderIcon,
    plan: 'Pro',
  },
  {
    name: 'Desktop pull request review',
    description: 'Review files in inline or split diff modes with markdown/SVG previews and full comment actions: create, edit, reply, and delete.',
    icon: GitPullRequestIcon,
    plan: 'Pro',
  },
]
</script>

<template>
  <section id="features" class="py-24 sm:py-32">
    <div class="mx-auto max-w-7xl px-6 lg:px-8">
      <div class="mx-auto max-w-3xl text-center">
        <h2 class="text-base/7 font-semibold text-primary">Features</h2>
        <p class="mt-2 text-5xl font-semibold tracking-tight text-balance text-foreground sm:text-6xl">Everything in Free for local Git.<br />Pro for GitHub workflows.</p>
        <p class="mx-auto mt-6 max-w-2xl text-lg/8 text-muted-foreground">
          Start with local Git and keyboard-first review workflows, then unlock GitHub notifications, repository browsing, pull request reviews, and issues with Reviu Pro for $19/month.
        </p>
      </div>
      <div class="mx-auto mt-16 grid max-w-6xl grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-3">
        <article v-for="feature in features" :key="feature.name" class="rounded-2xl bg-background p-6 ring-1 ring-muted">
          <div class="flex items-center justify-between gap-4">
            <component :is="feature.icon" class="size-5 text-primary" aria-hidden="true" />
            <span
              class="inline-flex items-center rounded-full px-2.5 py-1 text-xs font-semibold ring-1 ring-inset"
              :class="feature.plan === 'Pro' ? 'bg-primary/10 text-primary ring-primary/20' : 'bg-foreground/5 text-foreground ring-foreground/15'"
            >
              {{ feature.plan }}
            </span>
          </div>
          <h3 class="mt-4 text-lg font-semibold text-foreground">{{ feature.name }}</h3>
          <p class="mt-2 text-sm/6 text-muted-foreground">{{ feature.description }}</p>
        </article>
      </div>
    </div>
  </section>
</template>
