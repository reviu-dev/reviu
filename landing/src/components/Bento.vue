<script setup lang="ts">
import {
  BellIcon,
  CommandIcon,
  GitCommitHorizontalIcon,
  GitPullRequestIcon,
} from 'lucide-vue-next'
import cmd_palette_dark from '../assets/app_screenshots/cmd_palette_dark.png'
import cmd_palette_light from '../assets/app_screenshots/cmd_palette_light.png'
</script>

<template>
  <div class="py-24 sm:py-32">
    <div class="mx-auto max-w-2xl px-6 lg:max-w-7xl lg:px-8">
      <h2 class="text-base/7 font-semibold text-primary">Built for serious Git workflows</h2>
      <p class="mt-2 max-w-6xl text-4xl font-semibold tracking-tight text-pretty text-foreground sm:text-5xl">Everything you need to review, ship,<br /> and collaborate from one desktop app</p>
      <div class="mt-10 grid grid-cols-1 gap-4 sm:mt-16 lg:grid-cols-6 lg:grid-rows-2">
        <div class="flex p-px lg:col-span-4">
          <div class="w-full overflow-hidden rounded-lg bg-background shadow-sm outline outline-black/5 max-lg:rounded-t-4xl lg:rounded-tl-4xl dark:shadow-none dark:outline-white/15">
            <img class="h-80 object-cover object-left dark:hidden" :src="cmd_palette_light.src" alt="Keyboard-first workflow preview" />
            <img class="h-80 object-cover object-left not-dark:hidden" :src="cmd_palette_dark.src" alt="Keyboard-first workflow preview" />
            <div class="p-10">
              <div class="flex items-center gap-2">
                <CommandIcon class="size-4 text-primary" aria-hidden="true" />
                <h3 class="text-sm/4 font-semibold text-primary">Keyboard-first</h3>
              </div>
              <p class="mt-2 text-lg font-medium tracking-tight text-foreground">Move faster than mouse-driven Git clients</p>
              <p class="mt-2 max-w-lg text-sm/6 text-muted-foreground">Command palette, quick file search, and commit/review flows designed for shortcuts first.</p>
            </div>
          </div>
        </div>
        <div class="flex p-px lg:col-span-2">
          <div class="w-full overflow-hidden rounded-lg bg-background shadow-sm outline outline-black/5 lg:rounded-tr-4xl dark:shadow-none dark:outline-white/15">
            <img class="h-80 object-cover dark:hidden" src="https://tailwindcss.com/plus-assets/img/component-images/bento-02-integrations.png" alt="GitHub notifications and repositories preview" />
            <img class="h-80 object-cover not-dark:hidden" src="https://tailwindcss.com/plus-assets/img/component-images/dark-bento-02-integrations.png" alt="GitHub notifications and repositories preview" />
            <div class="p-10">
              <div class="flex items-center gap-2">
                <BellIcon class="size-4 text-primary" aria-hidden="true" />
                <h3 class="text-sm/4 font-semibold text-primary">GitHub integration</h3>
              </div>
              <p class="mt-2 text-lg font-medium tracking-tight text-foreground">Notifications, repos, PRs, and issues in one place</p>
              <p class="mt-2 max-w-lg text-sm/6 text-muted-foreground">Track activity from your desktop and open the exact context you need instantly.</p>
            </div>
          </div>
        </div>
        <div class="flex p-px lg:col-span-2">
          <div class="w-full overflow-hidden rounded-lg bg-background shadow-sm outline outline-black/5 lg:rounded-bl-4xl dark:shadow-none dark:outline-white/15">
            <img class="h-80 object-cover dark:hidden" src="https://tailwindcss.com/plus-assets/img/component-images/bento-02-security.png" alt="Pull request review preview" />
            <img class="h-80 object-cover not-dark:hidden" src="https://tailwindcss.com/plus-assets/img/component-images/dark-bento-02-security.png" alt="Pull request review preview" />
            <div class="p-10">
              <div class="flex items-center gap-2">
                <GitPullRequestIcon class="size-4 text-primary" aria-hidden="true" />
                <h3 class="text-sm/4 font-semibold text-primary">PR review</h3>
              </div>
              <p class="mt-2 text-lg font-medium tracking-tight text-foreground">Inline and split diffs with full code context</p>
              <p class="mt-2 max-w-lg text-sm/6 text-muted-foreground">Review changed files with syntax highlighting, markdown/svg previews, and threaded comments.</p>
            </div>
          </div>
        </div>
        <div class="flex p-px lg:col-span-4">
          <div class="w-full overflow-hidden rounded-lg bg-background shadow-sm outline outline-black/5 max-lg:rounded-b-4xl lg:rounded-br-4xl dark:shadow-none dark:outline-white/15">
            <img class="h-80 object-cover object-left dark:hidden" src="https://tailwindcss.com/plus-assets/img/component-images/bento-02-performance.png" alt="Local Git operations preview" />
            <img class="h-80 object-cover object-left not-dark:hidden" src="https://tailwindcss.com/plus-assets/img/component-images/dark-bento-02-performance.png" alt="Local Git operations preview" />
            <div class="p-10">
              <div class="flex items-center gap-2">
                <GitCommitHorizontalIcon class="size-4 text-primary" aria-hidden="true" />
                <h3 class="text-sm/4 font-semibold text-primary">Local Git power</h3>
              </div>
              <p class="mt-2 text-lg font-medium tracking-tight text-foreground">Stage hunks, amend commits, rebase, stash, cherry-pick</p>
              <p class="mt-2 max-w-lg text-sm/6 text-muted-foreground">Advanced local operations and conflict helpers built into a single editor-centric workflow.</p>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
