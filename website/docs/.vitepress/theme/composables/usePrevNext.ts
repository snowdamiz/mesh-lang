import { computed } from 'vue'
import { useData } from 'vitepress'
import { isActive, useSidebar, type SidebarItem } from './useSidebar'

interface PagerLink { text: string; link: string }
type PageItem = SidebarItem & PagerLink

const flatten = (items: SidebarItem[]): SidebarItem[] => items.flatMap((item) => [item, ...flatten(item.items ?? [])])

/** The sidebar pages before and after this one, skipping links marked includeInFooter: false. */
export function usePrevNext() {
  const { page, frontmatter } = useData()
  const { sidebar } = useSidebar()

  return computed(() => {
    const seen = new Set<string>()
    const pages = flatten(sidebar.value).filter((item): item is PageItem => {
      if (!item.text || !item.link || item.includeInFooter === false) return false
      const path = item.link.replace(/[?#].*$/, '')
      if (seen.has(path)) return false
      seen.add(path)
      return true
    })
    const index = pages.findIndex((item) => isActive(page.value.relativePath, item.link))
    const at = (i: number): PagerLink | undefined =>
      index === -1 || !pages[i] ? undefined : { text: pages[i].docFooterText ?? pages[i].text, link: pages[i].link }
    return {
      prev: frontmatter.value.prev === false ? undefined : at(index - 1),
      next: frontmatter.value.next === false ? undefined : at(index + 1),
    }
  })
}
