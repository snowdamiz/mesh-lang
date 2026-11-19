import { onMounted, onUnmounted, ref } from 'vue'
import { onContentUpdated } from 'vitepress'

const SELECTOR = '.docs-content :is(h2, h3)[id]'

/** The id of the last heading above the top 28% of the viewport. */
export function useActiveHeading() {
  const activeId = ref('')
  let frame = 0

  function sync() {
    frame = 0
    const line = window.innerHeight * 0.28
    const headings = document.querySelectorAll<HTMLElement>(SELECTOR)
    let current = headings[0]?.id ?? ''
    for (const heading of headings) {
      if (heading.getBoundingClientRect().top >= line) break
      current = heading.id
    }
    activeId.value = current
  }
  const onScroll = () => { if (!frame) frame = requestAnimationFrame(sync) }

  onMounted(() => {
    sync()
    window.addEventListener('scroll', onScroll, { passive: true })
  })
  onContentUpdated(sync)
  onUnmounted(() => {
    window.removeEventListener('scroll', onScroll)
    cancelAnimationFrame(frame)
  })

  return { activeId }
}
