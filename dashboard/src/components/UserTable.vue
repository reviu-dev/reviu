<script setup lang="ts">
import type { UserWithRole } from 'better-auth/client/plugins'
import { useAsyncState, watchDebounced } from '@vueuse/core'
import { MoreHorizontal, Search, UserPlus } from 'lucide-vue-next'
import { computed, ref } from 'vue'
import { toast } from 'vue-sonner'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
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
import { getInitials } from '@/lib/utils'

const emit = defineEmits<{
  createUser: []
  editUser: [user: UserWithRole]
  viewSessions: [userId: string]
  banUser: [user: UserWithRole]
  unbanUser: [userId: string]
  deleteUser: [user: UserWithRole]
  changeRole: [user: UserWithRole]
  changePassword: [user: UserWithRole]
}>()

const currentPage = ref(1)
const pageSize = ref(10)
const searchValue = ref('')
const searchField = ref<'email' | 'name'>('email')
const sortBy = ref('createdAt')
const sortDirection = ref<'asc' | 'desc'>('desc')
const isInitialLoad = ref(true)

const { state: usersData, execute: refetchUsers } = useAsyncState(
  async () => {
    const offset = (currentPage.value - 1) * pageSize.value
    const result = await authClient.admin.listUsers({
      query: {
        limit: pageSize.value,
        offset,
        searchValue: searchValue.value || undefined,
        searchField: searchField.value,
        sortBy: sortBy.value,
        sortDirection: sortDirection.value,
      },
    })

    if (result.error) {
      toast.error('Failed to fetch users')
      return null
    }

    isInitialLoad.value = false
    return result.data
  },
  null,
  {
    immediate: true,
    resetOnExecute: false,
  },
)

const users = computed(() => usersData.value?.users || [])
const totalUsers = computed(() => usersData.value?.total || 0)
const totalPages = computed(() => Math.ceil(totalUsers.value / pageSize.value))

watchDebounced(
  [currentPage, pageSize, searchValue, searchField, sortBy, sortDirection],
  () => {
    refetchUsers()
  },
  { debounce: 300 },
)

function formatDate(date: string | Date) {
  return new Date(date).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

function getRoleBadgeVariant(role: string | undefined) {
  return role === 'admin' ? 'default' : 'secondary'
}

function nextPage() {
  if (currentPage.value < totalPages.value) {
    currentPage.value++
  }
}

function prevPage() {
  if (currentPage.value > 1) {
    currentPage.value--
  }
}

defineExpose({
  refetchUsers,
})
</script>

<template>
  <div class="space-y-4">
    <!-- Header with search and create button -->
    <div class="flex items-center justify-between gap-2">
      <div class="flex items-center gap-2 flex-1 max-w-md">
        <div class="relative flex-1">
          <Search class="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            v-model="searchValue"
            placeholder="Search users..."
            class="pl-9"
          />
        </div>
        <Select v-model="searchField">
          <SelectTrigger class="w-[120px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="email">
              Email
            </SelectItem>
            <SelectItem value="name">
              Name
            </SelectItem>
          </SelectContent>
        </Select>
      </div>
      <Button @click="emit('createUser')">
        <UserPlus class="mr-2 h-4 w-4" />
        Create User
      </Button>
    </div>

    <!-- Table -->
    <div class="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>
              User
            </TableHead>
            <TableHead>
              Email
            </TableHead>
            <TableHead>
              Role
            </TableHead>
            <TableHead>
              Status
            </TableHead>
            <TableHead>
              Created
            </TableHead>
            <TableHead class="w-[70px]">
              Actions
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <template v-if="isInitialLoad">
            <TableRow v-for="i in pageSize" :key="i">
              <TableCell><Skeleton class="h-10 w-full" /></TableCell>
              <TableCell><Skeleton class="h-4 w-full" /></TableCell>
              <TableCell><Skeleton class="h-4 w-16" /></TableCell>
              <TableCell><Skeleton class="h-4 w-16" /></TableCell>
              <TableCell><Skeleton class="h-4 w-24" /></TableCell>
              <TableCell><Skeleton class="h-8 w-8" /></TableCell>
            </TableRow>
          </template>
          <template v-else-if="users.length === 0">
            <TableRow>
              <TableCell colspan="6" class="text-center text-muted-foreground">
                No users found
              </TableCell>
            </TableRow>
          </template>
          <template v-else>
            <TableRow v-for="user in users" :key="user.id">
              <TableCell>
                <div class="flex items-center gap-3">
                  <Avatar>
                    <AvatarFallback>
                      {{ getInitials(user.name) }}
                    </AvatarFallback>
                  </Avatar>
                  <div>
                    <div class="font-medium">
                      {{ user.name }}
                    </div>
                  </div>
                </div>
              </TableCell>
              <TableCell>
                <div class="text-sm text-muted-foreground">
                  {{ user.email }}
                </div>
              </TableCell>
              <TableCell>
                <Badge :variant="getRoleBadgeVariant(user.role)">
                  {{ user.role || 'user' }}
                </Badge>
              </TableCell>
              <TableCell>
                <Badge v-if="user.banned" variant="destructive">
                  Banned
                </Badge>
                <Badge v-else variant="outline">
                  Active
                </Badge>
              </TableCell>
              <TableCell>
                <div class="text-sm text-muted-foreground">
                  {{ formatDate(user.createdAt) }}
                </div>
              </TableCell>
              <TableCell>
                <DropdownMenu>
                  <DropdownMenuTrigger as-child>
                    <Button variant="ghost" size="icon">
                      <MoreHorizontal class="h-4 w-4" />
                      <span class="sr-only">Open menu</span>
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuLabel>Actions</DropdownMenuLabel>
                    <DropdownMenuItem @click="emit('editUser', user)">
                      Edit User
                    </DropdownMenuItem>
                    <DropdownMenuItem @click="emit('changeRole', user)">
                      Change Role
                    </DropdownMenuItem>
                    <DropdownMenuItem @click="emit('changePassword', user)">
                      Change Password
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem @click="emit('viewSessions', user.id)">
                      View Sessions
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      v-if="!user.banned"
                      class="text-orange-600"
                      @click="emit('banUser', user)"
                    >
                      Ban User
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      v-else
                      class="text-green-600"
                      @click="emit('unbanUser', user.id)"
                    >
                      Unban User
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      class="text-destructive"
                      @click="emit('deleteUser', user)"
                    >
                      Delete User
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </TableCell>
            </TableRow>
          </template>
        </TableBody>
      </Table>
    </div>

    <!-- Pagination -->
    <div class="flex items-center justify-between">
      <div class="text-sm text-muted-foreground">
        Showing {{ users.length > 0 ? (currentPage - 1) * pageSize + 1 : 0 }} to
        {{ Math.min(currentPage * pageSize, totalUsers) }} of {{ totalUsers }} users
      </div>
      <div class="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="currentPage === 1"
          @click="prevPage"
        >
          Previous
        </Button>
        <div class="text-sm">
          Page {{ currentPage }} of {{ totalPages || 1 }}
        </div>
        <Button
          variant="outline"
          size="sm"
          :disabled="currentPage >= totalPages"
          @click="nextPage"
        >
          Next
        </Button>
      </div>
    </div>
  </div>
</template>
