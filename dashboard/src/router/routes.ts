import type { RouteRecordRaw } from 'vue-router'

export const routes: RouteRecordRaw[] = [
  {
    name: 'Home',
    path: '/',
    redirect: { name: 'GithubCache' },
  },
  {
    name: 'Signin',
    path: '/signin',
    component: () => import('@/pages/Signin.vue'),
    meta: {
      layout: 'auth',
    },
  },
  {
    name: 'GithubCache',
    path: '/github-cache',
    component: () => import('@/pages/GithubCache.vue'),
  },
  {
    name: 'ClientAnalytics',
    path: '/client-analytics',
    component: () => import('@/pages/ClientAnalytics.vue'),
  },
  {
    name: 'ManageUsers',
    path: '/manage-users',
    component: () => import('@/pages/ManageUsers.vue'),
  },
]
