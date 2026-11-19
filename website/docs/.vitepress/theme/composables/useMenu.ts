import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vitepress'

/** The two narrow-screen menus. One ref holds both, so opening one closes the other. */
export type Menu = 'site' | 'docs'

const PANEL: Record<Menu, string> = { site: 'primary-nav', docs: 'docs-sidebar' }
const open = ref<Menu | null>(null)
/** The page the open menu was opened on. The router has already moved location by the time a link's click handler runs. */
let openedOn = ''
/** Set when the router takes a menu link to another page, so focus can move to that page once it renders. */
let followed = false

const toggleOf = (menu: Menu) => document.querySelector<HTMLElement>(`[aria-controls="${PANEL[menu]}"]`)

/** Closes `menu` if it is open and hands focus back to the button that opened it. */
function closeMenu(menu: Menu) {
  if (open.value !== menu) return
  open.value = null
  toggleOf(menu)?.focus()
}

function openMenu(menu: Menu) {
  open.value = menu
  openedOn = location.pathname + location.search
}

/** Focuses the page's main region without leaving it in the tab order. The router moves no focus on its own. */
export function focusMain() {
  const main = document.getElementById('main')
  if (!main) return
  main.setAttribute('tabindex', '-1')
  main.addEventListener('blur', () => main.removeAttribute('tabindex'), { once: true })
  main.focus({ preventScroll: true })
}

/**
 * A link to the page already shown goes nowhere, so focus returns to the toggle. Any other link hides
 * itself with the menu; when the router takes it (it prevents the default), focus moves to the new page.
 */
function closeFromLink(menu: Menu, e: MouseEvent) {
  const link = e.currentTarget as HTMLAnchorElement
  if (link.origin === location.origin && link.pathname + link.search === openedOn) return closeMenu(menu)
  if (open.value !== menu) return
  open.value = null
  followed = e.defaultPrevented
}

export function useMenu(menu: Menu) {
  const isOpen = computed(() => open.value === menu)
  return {
    isOpen,
    panelId: PANEL[menu],
    toggle: () => (isOpen.value ? closeMenu(menu) : openMenu(menu)),
    onLinkClick: (e: MouseEvent) => closeFromLink(menu, e),
  }
}

/**
 * Behaviour both menus share; the layout mounts it once. Escape closes the open
 * menu, navigation and the wide breakpoint close it, a page opened from a menu
 * link takes focus at its main region once rendered, the docs overlay locks page
 * scroll, and an opened menu takes focus at its current page link (or its first link).
 */
export function useMenuEffects() {
  const route = useRoute()
  watch(() => route.path, () => {
    open.value = null
    if (followed) focusMain()
    followed = false
  }, { flush: 'post' })
  watch(open, async (menu) => {
    document.documentElement.classList.toggle('menu-lock', menu === 'docs')
    if (!menu) return
    await nextTick()
    const panel = document.getElementById(PANEL[menu])
    ;(panel?.querySelector<HTMLElement>('[aria-current="page"]') ?? panel?.querySelector<HTMLElement>('a'))?.focus()
  })

  const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && open.value) closeMenu(open.value) }
  const onWide = (e: MediaQueryListEvent) => { if (e.matches) open.value = null }
  let wide: MediaQueryList | undefined
  onMounted(() => {
    window.addEventListener('keydown', onKey)
    wide = window.matchMedia('(min-width: 981px)')
    wide.addEventListener('change', onWide)
  })
  onBeforeUnmount(() => {
    window.removeEventListener('keydown', onKey)
    wide?.removeEventListener('change', onWide)
  })
}
