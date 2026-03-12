<script setup lang="ts">
import { ref } from 'vue'
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

import { authClient } from '@/lib/auth-client'

defineProps<{
  open: boolean
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  'success': []
}>()

const form = ref({
  email: '',
  password: '',
  name: '',
  role: 'user',
})

const isLoading = ref(false)

async function handleSubmit() {
  if (!form.value.email || !form.value.password || !form.value.name) {
    toast.error('Validation Error', {
      description: 'Please fill in all required fields',
    })
    return
  }

  isLoading.value = true

  const result = await authClient.admin.createUser({
    email: form.value.email,
    password: form.value.password,
    name: form.value.name,
    role: form.value.role as 'user' | 'admin',
  })

  isLoading.value = false

  if (result.error) {
    toast.error('Failed to create user', {
      description: result.error.message,
    })
    return
  }

  toast.success('User created successfully')
  resetForm()
  emit('update:open', false)
  emit('success')
}

function resetForm() {
  form.value = {
    email: '',
    password: '',
    name: '',
    role: 'user',
  }
}

function handleOpenChange(open: boolean) {
  emit('update:open', open)
  if (!open) {
    resetForm()
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="sm:max-w-[425px]">
      <DialogHeader>
        <DialogTitle>Create New User</DialogTitle>
        <DialogDescription>
          Create a new user account with email and password.
        </DialogDescription>
      </DialogHeader>
      <div class="grid gap-4 py-4">
        <div class="grid gap-2">
          <Label for="name">Name</Label>
          <Input
            id="name"
            v-model="form.name"
            placeholder="John Doe"
            :disabled="isLoading"
          />
        </div>
        <div class="grid gap-2">
          <Label for="email">Email</Label>
          <Input
            id="email"
            v-model="form.email"
            type="email"
            placeholder="user@example.com"
            :disabled="isLoading"
          />
        </div>
        <div class="grid gap-2">
          <Label for="password">Password</Label>
          <Input
            id="password"
            v-model="form.password"
            type="password"
            placeholder="••••••••"
            :disabled="isLoading"
          />
        </div>
        <div class="grid gap-2">
          <Label for="role">Role</Label>
          <Select v-model="form.role" :disabled="isLoading">
            <SelectTrigger>
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
          {{ isLoading ? 'Creating...' : 'Create User' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
