<script setup lang="ts">
import { useAsyncState } from '@vueuse/core'
import { watch } from 'vue'
import { toast } from 'vue-sonner'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { authClient } from '@/lib/auth-client'

const props = defineProps<{
  open: boolean
  userId: string | null
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  'success': []
}>()

const { state: sessions, isLoading, execute: refetchSessions } = useAsyncState(
  async () => {
    if (!props.userId)
      return []

    const result = await authClient.admin.listUserSessions({
      userId: props.userId,
    })

    if (result.error) {
      toast.error('Failed to fetch sessions', {
        description: result.error.message,
      })
      return []
    }

    return result.data?.sessions || []
  },
  [],
  {
    immediate: false,
  },
)

watch(
  () => props.userId,
  (userId) => {
    if (!userId) {
      return
    }

    refetchSessions()
  },
  { immediate: true },
)

async function revokeSession(sessionToken: string) {
  const result = await authClient.admin.revokeUserSession({
    sessionToken,
  })

  if (result.error) {
    toast.error('Failed to revoke session', {
      description: result.error.message,
    })
    return
  }

  toast.success('Session revoked successfully')
  refetchSessions()
  emit('success')
}

async function revokeAllSessions() {
  if (!props.userId)
    return

  const result = await authClient.admin.revokeUserSessions({
    userId: props.userId,
  })

  if (result.error) {
    toast.error('Failed to revoke all sessions', {
      description: result.error.message,
    })
    return
  }

  toast.success('All sessions revoked successfully')
  refetchSessions()
  emit('success')
}

function formatDate(date: string | Date) {
  return new Date(date).toLocaleString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function handleOpenChange(open: boolean) {
  emit('update:open', open)
}
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="sm:max-w-[800px]">
      <DialogHeader>
        <DialogTitle>User Sessions</DialogTitle>
        <DialogDescription>
          View and manage active sessions for this user.
        </DialogDescription>
      </DialogHeader>
      <div class="space-y-4 py-4 overflow-x-auto">
        <div class="flex justify-end">
          <Button
            variant="destructive"
            size="sm"
            :disabled="isLoading || !sessions || sessions.length === 0"
            @click="revokeAllSessions"
          >
            Revoke All Sessions
          </Button>
        </div>
        <div class="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>
                  IP Address
                </TableHead>
                <TableHead>
                  User Agent
                </TableHead>
                <TableHead>
                  Created
                </TableHead>
                <TableHead>
                  Expires
                </TableHead>
                <TableHead class="w-[100px]">
                  Actions
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <template v-if="isLoading">
                <TableRow v-for="i in 3" :key="i">
                  <TableCell><Skeleton class="h-4 w-full" /></TableCell>
                  <TableCell><Skeleton class="h-4 w-full" /></TableCell>
                  <TableCell><Skeleton class="h-4 w-24" /></TableCell>
                  <TableCell><Skeleton class="h-4 w-24" /></TableCell>
                  <TableCell><Skeleton class="h-8 w-16" /></TableCell>
                </TableRow>
              </template>
              <template v-else-if="!sessions || sessions.length === 0">
                <TableRow>
                  <TableCell colspan="5" class="text-center text-muted-foreground">
                    No active sessions found
                  </TableCell>
                </TableRow>
              </template>
              <template v-else>
                <TableRow v-for="session in sessions" :key="session.id">
                  <TableCell>
                    <div class="text-sm">
                      {{ session.ipAddress || 'N/A' }}
                    </div>
                  </TableCell>
                  <TableCell>
                    <div class="text-sm text-muted-foreground max-w-[200px] truncate">
                      {{ session.userAgent || 'N/A' }}
                    </div>
                  </TableCell>
                  <TableCell>
                    <div class="text-sm text-muted-foreground">
                      {{ formatDate(session.createdAt) }}
                    </div>
                  </TableCell>
                  <TableCell>
                    <div class="text-sm text-muted-foreground">
                      {{ formatDate(session.expiresAt) }}
                    </div>
                  </TableCell>
                  <TableCell>
                    <Button
                      variant="ghost"
                      size="sm"
                      @click="revokeSession(session.token)"
                    >
                      Revoke
                    </Button>
                  </TableCell>
                </TableRow>
              </template>
            </TableBody>
          </Table>
        </div>
      </div>
    </DialogContent>
  </Dialog>
</template>
