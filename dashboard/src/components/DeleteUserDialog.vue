<script setup lang="ts">
import type { UserWithRole } from 'better-auth/client/plugins'
import { ref } from 'vue'
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
import { authClient } from '@/lib/auth-client'

const props = defineProps<{
  open: boolean
  user: UserWithRole | null
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  'success': []
}>()

const isLoading = ref(false)

async function handleDelete() {
  if (!props.user)
    return

  isLoading.value = true

  const result = await authClient.admin.removeUser({
    userId: props.user.id,
  })

  isLoading.value = false

  if (result.error) {
    toast.error('Failed to delete user', {
      description: result.error.message,
    })
    return
  }

  toast.success('User deleted successfully')
  emit('update:open', false)
  emit('success')
}

function handleOpenChange(open: boolean) {
  emit('update:open', open)
}
</script>

<template>
  <AlertDialog :open="open" @update:open="handleOpenChange">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete User</AlertDialogTitle>
        <AlertDialogDescription>
          Are you sure you want to permanently delete {{ user?.name || 'this user' }}?
          This action cannot be undone. All user data will be permanently removed from the database.
        </AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel :disabled="isLoading">
          Cancel
        </AlertDialogCancel>
        <AlertDialogAction
          :disabled="isLoading"
          class="bg-destructive text-destructive-foreground hover:bg-destructive/90"
          @click.prevent="handleDelete"
        >
          {{ isLoading ? 'Deleting...' : 'Delete User' }}
        </AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
