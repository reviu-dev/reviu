import { createGlobalState, useAsyncState, useLocalStorage } from '@vueuse/core'
import { computed, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { betterAuthClient } from '@/lib/auth-client'
import { usersClient } from '@/services/user'

interface User {
  subscription: {
    portalUrl: string
    activeSubscription: {
      id: string
      createdAt: Date
      modifiedAt: Date | null
      customFieldData?:
        | { [k: string]: string | number | boolean | Date | null }
        | undefined
      metadata: { [k: string]: string | number | boolean }
      status: 'active' | 'trialing'
      amount: number
      currency: string
      currentPeriodStart: Date
      currentPeriodEnd: Date | null
      trialStart: Date | null
      trialEnd: Date | null
      cancelAtPeriodEnd: boolean
      canceledAt: Date | null
      startedAt: Date | null
      endsAt: Date | null
      productId: string
      discountId: string | null
    } | null
  }
  githubLogin: string | null
  id: string
  name: string
  email: string
  emailVerified: boolean
  image?: string | null | undefined
  role: 'user' | 'admin'
}

export const LS_BEARER_KEY = 'bearer_token'

export const useAuthStore = createGlobalState(
  <UserRequired extends boolean = false>() => {
    const me = ref<User>()
    const token = useLocalStorage<string>(LS_BEARER_KEY, null)
    const firstLoad = ref(true)

    const router = useRouter()

    function handleUserRedirect(user?: User) {
      const authRoutes = ['/signin']
      const currentRoutePath = new URL(window.location.href).pathname

      if (user?.role === 'admin' && authRoutes.includes(currentRoutePath)) {
        return router.push({ name: 'Home' })
      }

      if (!user && !authRoutes.includes(currentRoutePath)) {
        return router.push({ name: 'Signin' })
      }
    }

    function resetState() {
      me.value = undefined
      token.value = null
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
          firstLoad.value = false

          if (!res?.ok) {
            handleUserRedirect()
            resetState()
            return
          }

          const user = await res.json()

          me.value = user as User

          handleUserRedirect(me.value)
        },
      },
    )

    watch(token, () => {
      if (!token.value) {
        return
      }

      refetchMe()
    })

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
