import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import { routes } from './routes'

export const router = createRouter({
  history: createWebHistory(),
  routes,
})

router.beforeEach((to, from, next) => {
  const { me } = useAuthStore()

  if (to.name !== 'Signin' && me.value?.role !== 'admin') {
    next({ name: 'Signin' })
  }
  else if (to.name === 'Signin' && me.value) {
    next({ name: 'Home' })
  }
  else {
    next()
  }
})
