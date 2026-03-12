import type { RouteRecordRaw } from 'vue-router'

export const routes: RouteRecordRaw[] = [
  {
    name: 'Home',
    path: '/',
    redirect: { name: 'GithubCache' },
  },
  {
    name: 'GithubCache',
    path: '/github-cache',
    component: () => import('@/pages/GithubCache.vue'),
  },
  {
    name: 'ManageUsers',
    path: '/manage-users',
    component: () => import('@/pages/ManageUsers.vue'),
  },
]
