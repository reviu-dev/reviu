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

const form = ref({
  name: '',
  email: '',
})

const isLoading = ref(false)

watch(
  () => props.user,
  (user) => {
    if (user) {
      form.value = {
        name: user.name || '',
        email: user.email || '',
      }
    }
  },
  { immediate: true },
)

async function handleSubmit() {
  if (!props.user)
    return

  if (!form.value.name || !form.value.email) {
    toast.error('Validation Error', {
      description: 'Please fill in all required fields',
    })
    return
  }

  isLoading.value = true

  const result = await authClient.admin.updateUser({
    userId: props.user.id,
    data: {
      name: form.value.name,
      email: form.value.email,
    },
  })

  isLoading.value = false

  if (result.error) {
    toast.error('Failed to update user', {
      description: result.error.message,
    })
    return
  }

  toast.success('User updated successfully')
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
        <DialogTitle>Edit User</DialogTitle>
        <DialogDescription>
          Update user information.
        </DialogDescription>
      </DialogHeader>
      <div class="grid gap-4 py-4">
        <div class="grid gap-2">
          <Label for="edit-name">Name</Label>
          <Input
            id="edit-name"
            v-model="form.name"
            placeholder="John Doe"
            :disabled="isLoading"
          />
        </div>
        <div class="grid gap-2">
          <Label for="edit-email">Email</Label>
          <Input
            id="edit-email"
            v-model="form.email"
            type="email"
            placeholder="user@example.com"
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
          {{ isLoading ? 'Saving...' : 'Save Changes' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
