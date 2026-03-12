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
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { authClient } from '@/lib/auth-client'

const props = defineProps<{
  open: boolean
  user: UserWithRole | null
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  'success': []
}>()

const selectedRole = ref('user')
const isLoading = ref(false)

watch(
  () => props.user,
  (user) => {
    if (user) {
      selectedRole.value = user.role || 'user'
    }
  },
  { immediate: true },
)

async function handleSubmit() {
  if (!props.user)
    return

  isLoading.value = true

  const result = await authClient.admin.setRole({
    userId: props.user.id,
    role: selectedRole.value as 'user' | 'admin',
  })

  isLoading.value = false

  if (result.error) {
    toast.error('Failed to change role', {
      description: result.error.message,
    })
    return
  }

  toast.success('User role updated successfully')
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
        <DialogTitle>Change User Role</DialogTitle>
        <DialogDescription>
          Update the role for {{ user?.name || 'this user' }}.
        </DialogDescription>
      </DialogHeader>
      <div class="grid gap-4 py-4">
        <div class="grid gap-2">
          <Label for="role-select">Role</Label>
          <Select v-model="selectedRole" :disabled="isLoading">
            <SelectTrigger id="role-select">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="user">
                User
              </SelectItem>
              <SelectItem value="admin">
                Admin
              </SelectItem>
            </SelectContent>
          </Select>
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
