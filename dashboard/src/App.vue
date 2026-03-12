<script setup lang="ts">
import { Toaster } from '@/components/ui/sonner'
import MainLayout from '@/layouts/MainLayout.vue'
import 'vue-sonner/style.css'
</script>

<template>
  <MainLayout>
    <RouterView />
    <Toaster />
  </MainLayout>
</template>
