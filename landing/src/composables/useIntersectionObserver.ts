import { onMounted, onUnmounted, ref, useTemplateRef } from 'vue'

export function useIntersectionObserver(threshold = 0.1) {
  const isVisible = ref(false)
  const targetRef = useTemplateRef<HTMLElement | null>('targetRef')
  
  let observer: IntersectionObserver | null = null

  onMounted(() => {
    if (!targetRef.value) return

    observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          isVisible.value = true
          observer?.disconnect()
        }
      },
      { threshold }
    )

    observer.observe(targetRef.value)
  })

  onUnmounted(() => {
    observer?.disconnect()
  })

  return { isVisible }
}
