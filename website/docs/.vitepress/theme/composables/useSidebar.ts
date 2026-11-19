import { computed } from 'vue'
import { useData } from 'vitepress'

export interface SidebarItem {
  text?: string
  link?: string
  items?: SidebarItem[]
  collapsed?: boolean
  base?: string
  docFooterText?: string
  includeInFooter?: boolean
}

export function useSidebar() {
  const { theme, page, frontmatter } = useData()

  const sidebar = computed<SidebarItem[]>(() => {
    const sidebarConfig = theme.value.sidebar
    if (!sidebarConfig) return []
    if (Array.isArray(sidebarConfig)) return sidebarConfig

    // Multi-sidebar: find matching path by longest prefix
    const relativePath = page.value.relativePath
    const path = ensureStartingSlash(relativePath)
    const dir = Object.keys(sidebarConfig)
      .sort((a, b) => b.split('/').length - a.split('/').length)
      .find((dir) => path.startsWith(ensureStartingSlash(dir)))
    return dir ? sidebarConfig[dir] : []
  })

  const hasSidebar = computed(() => {
    return (
      frontmatter.value.sidebar !== false &&
      sidebar.value.length > 0 &&
      frontmatter.value.layout !== 'home'
    )
  })

  return { sidebar, hasSidebar }
}

function ensureStartingSlash(path: string): string {
  return path.startsWith('/') ? path : `/${path}`
}

/** Whether a sidebar link is the current page. */
export function isActive(currentPath: string, matchPath?: string): boolean {
  if (!matchPath) return false
  return normalize(currentPath) === normalize(matchPath)
}

function normalize(path: string): string {
  return ensureStartingSlash(path.replace(/(index)?\.(md|html)$/, '').replace(/\/$/, ''))
}
