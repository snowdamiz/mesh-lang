import { computed } from 'vue'
import { useData } from 'vitepress'
import { isActive } from './useSidebar'

interface FlatLink {
  text: string
  link: string
  docFooterText?: string
}

export function usePrevNext() {
  const { page, theme, frontmatter } = useData()

  return computed(() => {
    const sidebarConfig = theme.value.sidebar
    if (!sidebarConfig) return { prev: undefined, next: undefined }

    // Resolve current sidebar
    const relativePath = page.value.relativePath
    const sidebar = resolveSidebar(sidebarConfig, relativePath)

    // Flatten all links from sidebar
    const links = flattenSidebarLinks(sidebar)
    const candidates = uniqBy(links, (l) => l.link.replace(/[?#].*$/, ''))

    // Find current page index
    const index = candidates.findIndex((link) =>
      isActive(page.value.relativePath, link.link),
    )

    return {
      prev:
        frontmatter.value.prev === false
          ? undefined
          : {
              text:
                candidates[index - 1]?.docFooterText ??
                candidates[index - 1]?.text,
              link: candidates[index - 1]?.link,
            },
      next:
        frontmatter.value.next === false
          ? undefined
          : {
              text:
                candidates[index + 1]?.docFooterText ??
                candidates[index + 1]?.text,
              link: candidates[index + 1]?.link,
            },
    }
  })
}

function flattenSidebarLinks(items: any[]): FlatLink[] {
  const links: FlatLink[] = []
  function recurse(items: any[]) {
    for (const item of items) {
      if (item.text && item.link) {
        links.push({
          text: item.text,
          link: item.link,
          docFooterText: item.docFooterText,
        })
      }
      if (item.items) recurse(item.items)
    }
  }
  recurse(items)
  return links
}

function resolveSidebar(sidebar: any, relativePath: string): any[] {
  if (Array.isArray(sidebar)) return sidebar
  const path = relativePath.startsWith('/')
    ? relativePath
    : `/${relativePath}`
  const dir = Object.keys(sidebar)
    .sort((a, b) => b.split('/').length - a.split('/').length)
    .find((d) => path.startsWith(d.startsWith('/') ? d : `/${d}`))
  return dir ? sidebar[dir] : []
}

function uniqBy<T>(arr: T[], fn: (item: T) => string): T[] {
  const seen = new Set<string>()
  return arr.filter((item) => {
    const k = fn(item)
    return seen.has(k) ? false : (seen.add(k), true)
  })
}
