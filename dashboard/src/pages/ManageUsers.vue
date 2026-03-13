<script setup lang="ts">
import type { UserWithRole } from 'better-auth/client/plugins'
import { ref, useTemplateRef } from 'vue'
import { toast } from 'vue-sonner'
import BanUserDialog from '@/components/BanUserDialog.vue'
import ChangePasswordDialog from '@/components/ChangePasswordDialog.vue'
import ChangeRoleDialog from '@/components/ChangeRoleDialog.vue'
import CreateUserDialog from '@/components/CreateUserDialog.vue'
import DeleteUserDialog from '@/components/DeleteUserDialog.vue'
import EditUserDialog from '@/components/EditUserDialog.vue'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import UserTable from '@/components/UserTable.vue'
import ViewSessionsDialog from '@/components/ViewSessionsDialog.vue'
import { betterAuthClient } from '@/lib/auth-client'

const createUserDialogOpen = ref(false)
const editUserDialogOpen = ref(false)
const changeRoleDialogOpen = ref(false)
const changePasswordDialogOpen = ref(false)
const banUserDialogOpen = ref(false)
const deleteUserDialogOpen = ref(false)
const viewSessionsDialogOpen = ref(false)

const selectedUser = ref<UserWithRole | null>(null)
const selectedUserId = ref<string | null>(null)

const userTableRef = useTemplateRef('userTableRef')

function handleCreateUser() {
  createUserDialogOpen.value = true
}

function handleEditUser(user: UserWithRole) {
  selectedUser.value = user
  editUserDialogOpen.value = true
}

function handleChangeRole(user: UserWithRole) {
  selectedUser.value = user
  changeRoleDialogOpen.value = true
}

function handleChangePassword(user: UserWithRole) {
  selectedUser.value = user
  changePasswordDialogOpen.value = true
}

function handleBanUser(user: UserWithRole) {
  selectedUser.value = user
  banUserDialogOpen.value = true
}

async function handleUnbanUser(userId: string) {
  const result = await betterAuthClient.admin.unbanUser({
    userId,
  })

  if (result.error) {
    toast.error('Failed to unban user', {
      description: result.error.message,
    })
    return
  }

  toast.success('User unbanned successfully')
  userTableRef.value?.refetchUsers()
}

function handleDeleteUser(user: UserWithRole) {
  selectedUser.value = user
  deleteUserDialogOpen.value = true
}

function handleViewSessions(userId: string) {
  selectedUserId.value = userId
  viewSessionsDialogOpen.value = true
}

function handleSuccess() {
  userTableRef.value?.refetchUsers()
}
</script>

<template>
  <div class="container mx-auto space-y-6">
    <div>
      <h1 class="text-3xl font-bold">
        Admin Dashboard
      </h1>
      <p class="text-muted-foreground mt-2">
        Manage users, roles, and permissions
      </p>
    </div>

    <Card>
      <CardHeader>
        <CardTitle>User Management</CardTitle>
        <CardDescription>
          View, create, edit, and manage all users in the system
        </CardDescription>
      </CardHeader>
      <CardContent>
        <UserTable
          ref="userTableRef"
          @create-user="handleCreateUser"
          @edit-user="handleEditUser"
          @view-sessions="handleViewSessions"
          @ban-user="handleBanUser"
          @unban-user="handleUnbanUser"
          @delete-user="handleDeleteUser"
          @change-role="handleChangeRole"
          @change-password="handleChangePassword"
        />
      </CardContent>
    </Card>

    <CreateUserDialog
      v-model:open="createUserDialogOpen"
      @success="handleSuccess"
    />

    <EditUserDialog
      v-model:open="editUserDialogOpen"
      :user="selectedUser"
      @success="handleSuccess"
    />

    <ChangeRoleDialog
      v-model:open="changeRoleDialogOpen"
      :user="selectedUser"
      @success="handleSuccess"
    />

    <ChangePasswordDialog
      v-model:open="changePasswordDialogOpen"
      :user="selectedUser"
      @success="handleSuccess"
    />

    <BanUserDialog
      v-model:open="banUserDialogOpen"
      :user="selectedUser"
      @success="handleSuccess"
    />

    <DeleteUserDialog
      v-model:open="deleteUserDialogOpen"
      :user="selectedUser"
      @success="handleSuccess"
    />

    <ViewSessionsDialog
      v-model:open="viewSessionsDialogOpen"
      :user-id="selectedUserId"
      @success="handleSuccess"
    />
  </div>
</template>
