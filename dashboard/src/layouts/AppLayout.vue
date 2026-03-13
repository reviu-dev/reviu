<script setup lang="ts">
import AppSidebar from '@/components/AppSidebar.vue'
import { SidebarProvider, SidebarTrigger } from '@/components/ui/sidebar'
import { Toaster } from '@/components/ui/sonner'
</script>

<template>
  <SidebarProvider>
    <AppSidebar />
    <main class="flex-1 container mx-auto px-4 py-6">
      <SidebarTrigger class="mb-4" />
      <RouterView />
      <Toaster />
    </main>
  </SidebarProvider>
</template>
