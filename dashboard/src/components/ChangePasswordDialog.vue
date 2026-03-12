<script setup lang="ts">
import type { UserWithRole } from 'better-auth/client/plugins'
import { ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
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

const newPassword = ref('')
const confirmPassword = ref('')
const isLoading = ref(false)

watch(
  () => props.open,
  (open) => {
    if (!open) {
      newPassword.value = ''
      confirmPassword.value = ''
    }
  },
)

async function handleSubmit() {
  if (!props.user)
    return

  if (!newPassword.value) {
    toast.error('Validation Error', {
      description: 'Please enter a new password',
    })
    return
  }

  if (newPassword.value !== confirmPassword.value) {
    toast.error('Validation Error', {
      description: 'Passwords do not match',
    })
    return
  }

  if (newPassword.value.length < 8) {
    toast.error('Validation Error', {
      description: 'Password must be at least 8 characters',
    })
    return
  }

  isLoading.value = true

  const result = await authClient.admin.setUserPassword({
    userId: props.user.id,
    newPassword: newPassword.value,
  })

  isLoading.value = false

  if (result.error) {
    toast.error('Failed to change password', {
      description: result.error.message,
    })
    return
  }

  toast.success('Password changed successfully')
  emit('update:open', false)
  emit('success')
}

function handleOpenChange(open: boolean) {
  emit('update:open', open)
}
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="sm:max-w-[425px]">
      <DialogHeader>
        <DialogTitle>Change Password</DialogTitle>
        <DialogDescription>
          Set a new password for {{ user?.name || 'this user' }}.
        </DialogDescription>
      </DialogHeader>
      <div class="grid gap-4 py-4">
        <div class="grid gap-2">
          <Label for="new-password">New Password</Label>
          <Input
            id="new-password"
            v-model="newPassword"
            type="password"
            placeholder="••••••••"
            :disabled="isLoading"
          />
        </div>
        <div class="grid gap-2">
          <Label for="confirm-password">Confirm Password</Label>
          <Input
            id="confirm-password"
            v-model="confirmPassword"
            type="password"
            placeholder="••••••••"
            :disabled="isLoading"
          />
        </div>
      </div>
      <DialogFooter>
        <Button
          variant="outline"
          :disabled="isLoading"
          @click="handleOpenChange(false)"
        >
          Cancel
        </Button>
        <Button :disabled="isLoading" @click="handleSubmit">
          {{ isLoading ? 'Changing...' : 'Change Password' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
