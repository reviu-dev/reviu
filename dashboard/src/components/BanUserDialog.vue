<script setup lang="ts">
import type { UserWithRole } from 'better-auth/client/plugins'
import { ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { authClient } from '@/lib/auth-client'

const props = defineProps<{
  open: boolean
  user: UserWithRole | null
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  'success': []
}>()

const banReason = ref('')
const banDays = ref<number | undefined>(undefined)
const isLoading = ref(false)

watch(
  () => props.open,
  (open) => {
    if (!open) {
      banReason.value = ''
      banDays.value = undefined
    }
  },
)

async function handleBan() {
  if (!props.user)
    return

  isLoading.value = true

  const banExpiresIn = banDays.value ? banDays.value * 24 * 60 * 60 : undefined

  const result = await authClient.admin.banUser({
    userId: props.user.id,
    banReason: banReason.value || undefined,
    banExpiresIn,
  })

  isLoading.value = false

  if (result.error) {
    toast.error('Failed to ban user', {
      description: result.error.message,
    })
    return
  }

  toast.success('User banned successfully')
  emit('update:open', false)
  emit('success')
}

function handleOpenChange(open: boolean) {
  emit('update:open', open)
}
</script>

<template>
  <AlertDialog :open="open" @update:open="handleOpenChange">
    <AlertDialogContent class="sm:max-w-[425px]">
      <AlertDialogHeader>
        <AlertDialogTitle>Ban User</AlertDialogTitle>
        <AlertDialogDescription>
          Are you sure you want to ban {{ user?.name || 'this user' }}? This will prevent them from signing in and revoke all their active sessions.
        </AlertDialogDescription>
      </AlertDialogHeader>
      <div class="grid gap-4 py-4">
        <div class="grid gap-2">
          <Label for="ban-reason">Reason (optional)</Label>
          <Input
            id="ban-reason"
            v-model="banReason"
            placeholder="Spamming, inappropriate behavior, etc."
            :disabled="isLoading"
          />
        </div>
        <div class="grid gap-2">
          <Label for="ban-days">Ban Duration (days, optional)</Label>
          <Input
            id="ban-days"
            v-model.number="banDays"
            type="number"
            placeholder="Leave empty for permanent ban"
            :disabled="isLoading"
          />
          <p class="text-xs text-muted-foreground">
            Leave empty for a permanent ban
          </p>
        </div>
      </div>
      <AlertDialogFooter>
        <AlertDialogCancel :disabled="isLoading">
          Cancel
        </AlertDialogCancel>
        <AlertDialogAction
          class="bg-destructive text-white hover:bg-destructive/90"
          :disabled="isLoading"
          @click.prevent="handleBan"
        >
          {{ isLoading ? 'Banning...' : 'Ban User' }}
        </AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
