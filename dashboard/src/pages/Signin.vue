<script setup lang="ts">
import { onMounted } from 'vue'
import { Button } from '@/components/ui/button'
import { betterAuthClient } from '@/lib/auth-client'
import { authClient } from '@/services/auth'
import { useAuthStore } from '@/stores/auth'

const { setToken, refetchMe } = useAuthStore()

onMounted(async () => {
  const queryParams = new URLSearchParams(window.location.search)
  const code = queryParams.get('code')

  if (code) {
    const res = await authClient.exchange.$post({ json: { code } })

    if (!res.ok) {
      console.error('Failed to exchange code for token')
      return
    }

    const { token } = await res.json()

    setToken(token)
    refetchMe()
  }
})

async function signIn() {
  await betterAuthClient.signIn.social({
    provider: 'github',
    callbackURL: '/auth/web/callback',
  })
}
</script>

<template>
  <div class="flex items-center justify-center h-screen">
    <Button @click="signIn">
      Sign in
    </Button>
  </div>
</template>
