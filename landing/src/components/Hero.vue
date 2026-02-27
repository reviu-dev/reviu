<script setup lang="ts">
import {
  GitBranchIcon,
  GitPullRequestIcon,
  KeyboardIcon,
} from 'lucide-vue-next'
import git_dark from '../assets/app_screenshots/git_dark.png'
import git_light from '../assets/app_screenshots/git_light.png'
</script>

<template>
  <div class="relative isolate overflow-hidden">
    <svg class="absolute inset-0 -z-10 size-full mask-[radial-gradient(100%_100%_at_top_right,white,transparent)] stroke-gray-200 dark:stroke-white/10" aria-hidden="true">
      <defs>
        <pattern id="983e3e4c-de6d-4c3f-8d64-b9761d1534cc" width="200" height="200" x="50%" y="-1" patternUnits="userSpaceOnUse">
          <path d="M.5 200V.5H200" fill="none" />
        </pattern>
      </defs>
      <svg x="50%" y="-1" class="overflow-visible fill-gray-50 dark:fill-gray-800/20">
        <path d="M-200 0h201v201h-201Z M600 0h201v201h-201Z M-400 600h201v201h-201Z M200 800h201v201h-201Z" stroke-width="0" />
      </svg>
      <rect width="100%" height="100%" stroke-width="0" fill="url(#983e3e4c-de6d-4c3f-8d64-b9761d1534cc)" />
    </svg>
    <div class="absolute top-10 left-[calc(50%-4rem)] -z-10 transform-gpu blur-3xl sm:left-[calc(50%-18rem)] lg:top-[calc(50%-30rem)] lg:left-48 xl:left-[calc(50%-24rem)]" aria-hidden="true">
      <div class="aspect-1108/632 w-277 bg-linear-to-r from-[#80caff] to-[#4f46e5] opacity-20" style="clip-path: polygon(73.6% 51.7%, 91.7% 11.8%, 100% 46.4%, 97.4% 82.2%, 92.5% 84.9%, 75.7% 64%, 55.3% 47.5%, 46.5% 49.4%, 45% 62.9%, 50.3% 87.2%, 21.3% 64.1%, 0.1% 100%, 5.4% 51.1%, 21.4% 63.9%, 58.9% 0.2%, 73.6% 51.7%)"></div>
    </div>
    <div class="mx-auto max-w-7xl px-6 pt-10 pb-24 sm:pb-32 lg:flex lg:px-8 lg:py-40">
      <div class="mx-auto max-w-2xl shrink-0 lg:mx-0 lg:pt-8">
        <p class="text-6xl font-semibold text-foreground ">Rev<span class="text-primary">iu</span></p>
        
        <div class="mt-24 sm:mt-32 lg:mt-16">
          <span class="inline-flex rounded-full bg-primary/10 px-3 py-1 text-sm/6 font-semibold text-primary ring-1 ring-primary/20 ring-inset dark:ring-primary/25">Git client + GitHub workflows</span>
        </div>
        <h1 class="mt-10 text-5xl font-semibold tracking-tight text-pretty text-foreground sm:text-7xl">A free Git client built for fast reviews.</h1>
        <p class="mt-8 text-lg font-medium text-pretty text-muted-foreground sm:text-xl/8">
          Fast desktop Git reviews editor with real-time diffs and syntax highlighting. Upgrade to Reviu Pro for GitHub notifications, repos, PRs, and issues.
        </p>
        <div class="mt-8 flex flex-wrap items-center gap-x-6 gap-y-2 text-sm/6 text-muted-foreground">
          <span class="inline-flex items-center gap-2">
            <KeyboardIcon class="size-4 text-primary" aria-hidden="true" />
            Keyboard-first
          </span>
          <span class="inline-flex items-center gap-2">
            <GitBranchIcon class="size-4 text-primary" aria-hidden="true" />
            Advanced local Git
          </span>
          <span class="inline-flex items-center gap-2">
            <GitPullRequestIcon class="size-4 text-primary" aria-hidden="true" />
            PR review in-app
          </span>
        </div>
        <p class="mt-6 text-sm/6 font-medium text-foreground">Reviu Pro: $20/month for GitHub integration.</p>
        <div id="download" class="mt-10 flex flex-wrap items-center gap-x-6 gap-y-3">
          <a href="#download" class="rounded-md bg-primary px-3.5 py-2.5 text-sm font-semibold text-white shadow-xs hover:bg-primary/80 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary">Download for macOS (Apple Silicon)</a>
          <a href="#pricing" class="text-sm/6 font-semibold text-foreground">See Pro pricing <span aria-hidden="true">→</span></a>
        </div>
      </div>
      <div class="mx-auto mt-16 flex max-w-2xl sm:mt-24 lg:mt-0 lg:mr-0 lg:ml-10 lg:max-w-none lg:flex-none xl:ml-32">
        <div class="max-w-3xl flex-none sm:max-w-5xl lg:max-w-none">
          <div class="-m-2 rounded-xl bg-gray-900/5 p-2 ring-1 ring-gray-900/10 ring-inset lg:-m-4 lg:rounded-2xl lg:p-4 dark:bg-white/2.5 dark:ring-white/10">
            <img :src="git_light.src" alt="Reviu desktop interface preview" width="2432" height="1442" class="w-304 rounded-[13px] bg-white/0 shadow-xl ring-1 ring-gray-900/10 dark:hidden" />
            <img :src="git_dark.src" alt="Reviu desktop interface preview" width="2432" height="1442" class="w-304 rounded-[13px] bg-white/5 shadow-2xl ring-1 ring-white/10 not-dark:hidden" />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
