// Durable job queue figures for the landing page, read from the recorded
// comparison CSV so the page cannot drift from the evidence.
import type { BenchmarkPanel } from './benchmarks'

export const JOBS_RUN = {
  dir: 'tools/durable-jobs-bench/results/2026-11-26-landing-summary',
  footnote: 'Median of 5–8 rotations, from submit to the last durable settlement. Two pinned CPUs, PostgreSQL 17.11 with TLS, fsync and synchronous commit.',
}

const ENGINES = ['Mesh', 'Oban', 'River', 'Apalis']
const RIVAL = 'Oban'

const PANELS: Omit<BenchmarkPanel, 'rows' | 'headline' | 'unit' | 'better' | 'note' | 'measure'>[] = [
  { id: 'short', label: 'Short', setup: '128 jobs with 1 ms handlers on 16 slots.' },
  { id: 'dense', label: 'Dense', setup: '1,024 no-op jobs on two runners of 32 slots.' },
  { id: 'large', label: 'Large', setup: '64 jobs with 512 KiB arguments on eight runners of 8 slots.' },
  { id: 'slow', label: 'Slow', setup: '512 jobs with 16 KiB arguments and 100 ms handlers on two runners of 32 slots.' },
]

const FIGURE = new Intl.NumberFormat('en', { maximumFractionDigits: 0 })
const RATIO = new Intl.NumberFormat('en', { minimumFractionDigits: 1, maximumFractionDigits: 1 })

const fail = (message: string): never => { throw new Error(`jobs benchmarks: ${message}`) }

export function jobsPanels(csv: string): BenchmarkPanel[] {
  const [head, ...lines] = csv.trim().split(/\r?\n/)
  const names = head.split(',')
  const col = (name: string) => { const i = names.indexOf(name); return i < 0 ? fail(`the CSV has no "${name}" column`) : i }
  const [workload, engine, median] = ['workload', 'engine', 'median_ms'].map(col)
  const rows = lines.map((line) => line.split(','))
  return PANELS.map((panel) => {
    const values = ENGINES.map((name) => {
      const found = rows.filter((r) => r[workload] === panel.id && r[engine] === name)
      if (found.length !== 1) fail(`workload "${panel.id}" needs one "${name}" row, found ${found.length}`)
      const value = Number(found[0][median])
      if (!Number.isFinite(value) || value <= 0) fail(`workload "${panel.id}", engine "${name}": "${found[0][median]}" is not a positive number`)
      return value
    })
    const mesh = values[0]
    const rival = values[ENGINES.indexOf(RIVAL)]
    if (rival <= mesh) fail(`workload "${panel.id}": Mesh no longer beats ${RIVAL}; update the landing copy`)
    const longest = Math.max(...values)
    return {
      ...panel,
      headline: FIGURE.format(mesh),
      unit: 'ms',
      better: 'lower',
      measure: `${RATIO.format(rival / mesh)}× faster than ${RIVAL}`,
      rows: values.map((value, i) => ({
        runtime: ENGINES[i],
        figure: `${FIGURE.format(value)} ms`,
        share: value / longest,
        highlighted: i === 0,
      })),
    }
  })
}
