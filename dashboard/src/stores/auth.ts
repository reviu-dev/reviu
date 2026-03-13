import type { AsyncReturnType } from 'type-fest'
import { createGlobalState, useAsyncState, useLocalStorage } from '@vueuse/core'
import { computed, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { betterAuthClient } from '@/lib/auth-client'
import { usersClient } from '@/services/user'

type MeResponse = AsyncReturnType<AsyncReturnType<typeof usersClient.me.$get>['json']>
export type User = NonNullable<AsyncReturnType<typeof usersClient.me.$get>>
export const JWT_KEY = 'bearer_token'

export const useAuthStore = createGlobalState(
  <UserRequired extends boolean = false>() => {
    const me = ref<User>()
    const token = useLocalStorage<string>(JWT_KEY, null)
    const firstLoad = ref(true)

    const router = useRouter()

    function handleUserRedirect(user?: User) {
      const authRoutes = ['/signin']
      const currentRoutePath = new URL(window.location.href).pathname

      if (user && authRoutes.includes(currentRoutePath)) {
        return router.push({ name: 'Home' })
      }

      if (!user && !authRoutes.includes(currentRoutePath)) {
        return router.push({ name: 'Signin' })
      }
    }

    function resetState() {
      me.value = undefined
      token.value = undefined
    }

    function setToken(t: string) {
      token.value = t
    }

    const { execute: refetchMe } = useAsyncState(
      () => usersClient.me.$get(),
      undefined,
      {
        resetOnExecute: false,
        onSuccess: async (res) => {
          if (!res?.ok) {
            firstLoad.value = false
            handleUserRedirect()
            resetState()
            return
          }

          const user = await res.json()

          me.value = user
          firstLoad.value = false

          handleUserRedirect(me.value)
        },
      },
    )

    async function signout() {
      await Promise.all([
        betterAuthClient.signOut(),
        handleUserRedirect(),
      ])

      resetState()
    }

    return {
      me: computed(() => me.value as UserRequired extends true ? User : User | undefined),
      token: computed(() => token.value),
      firstLoad: computed(() => firstLoad.value),
      refetchMe,
      signout,
      setToken,
    }
  },
)
