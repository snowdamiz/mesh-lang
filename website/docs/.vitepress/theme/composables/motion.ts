import type { Directive } from 'vue'

export const prefersReducedMotion = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches

export function isOnScreen(el: Element): boolean {
  const r = el.getBoundingClientRect()
  return r.top < window.innerHeight && r.bottom > 0
}

/** Runs `fn` once, the first time `el` is at least `threshold` visible. Returns a cancel function. */
export function whenVisible(el: Element, fn: () => void, threshold = 0.5): () => void {
  const io = new IntersectionObserver((entries) => {
    if (entries[0].isIntersecting) { io.disconnect(); fn() }
  }, { threshold })
  io.observe(el)
  return () => io.disconnect()
}

let revealer: IntersectionObserver | undefined

/**
 * Fades an element in as it scrolls into view. Elements already on screen at
 * mount, and every element under reduced motion, are left alone, so the
 * server-rendered page is never hidden.
 */
export const vReveal: Directive<HTMLElement> = {
  mounted(el) {
    if (prefersReducedMotion() || isOnScreen(el)) return
    revealer ??= new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue
        entry.target.classList.add('in')
        revealer!.unobserve(entry.target)
      }
    }, { rootMargin: '0px 0px -8% 0px' })
    el.classList.add('rv')
    revealer.observe(el)
  },
  unmounted(el) {
    revealer?.unobserve(el)
  },
}
