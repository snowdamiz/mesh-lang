const STEP: Record<string, (i: number, n: number) => number> = {
  ArrowRight: (i, n) => (i + 1) % n,
  ArrowLeft: (i, n) => (i + n - 1) % n,
  Home: () => 0,
  End: (_, n) => n - 1,
}

/**
 * Keyboard for a horizontal tablist with a roving tabindex: Left and Right
 * wrap, Home and End jump. Bind the returned handler to the tablist's keydown;
 * it selects the new tab by index and moves focus to it.
 */
export function useRovingTabs(select: (index: number) => void) {
  return (e: KeyboardEvent) => {
    const tab = (e.target as HTMLElement).closest<HTMLElement>('[role="tab"]')
    const move = STEP[e.key]
    if (!tab || !move) return
    const tabs = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('[role="tab"]')]
    const next = move(tabs.indexOf(tab), tabs.length)
    e.preventDefault()
    select(next)
    tabs[next].focus()
  }
}
