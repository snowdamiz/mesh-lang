export interface BenchmarkPanel {
  id: 'actors' | 'cpu' | 'recovery' | 'http'
  label: string
  headline: string
  unit: string
  measure: string
  setup: string
  better: 'higher' | 'lower'
  note?: string
  rows: { runtime: string; figure: string; share: number; highlighted: boolean }[]
}

const TRIALS = '5'

export const RUN = {
  dir: 'tools/performance/results/2026-11-19',
  trials: TRIALS,
  footnote: `Median of ${TRIALS} trials on a dedicated 4-vCPU server. Rust, Go and Elixir were measured a day earlier.`,
}

const RUNTIMES = ['Mesh', 'Rust/Tokio', 'Go', 'Elixir']
const HIGHLIGHTED = 'Mesh'

const KEY_COLUMNS = ['workload', 'cpus', 'pairs', 'offered_rps', 'metric_unit', 'blocks'] as const
const COLUMNS = [...KEY_COLUMNS, 'runtime', 'median'] as const

type Row = Record<(typeof COLUMNS)[number], string>
type RowKey = Partial<Record<(typeof KEY_COLUMNS)[number], string>>

type PanelSpec = Omit<BenchmarkPanel, 'rows' | 'headline' | 'unit'> & { csvRow: RowKey & { metric_unit: string }; shortUnit?: string }

const PANELS: PanelSpec[] = [
  {
    id: 'actors',
    label: 'Actors',
    csvRow: { workload: 'actor', cpus: '4', pairs: '64', metric_unit: 'M round trips/s' },
    shortUnit: 'M',
    better: 'higher',
    measure: 'message round trips per second',
    setup: 'Ping-pong between 64 actor pairs on 4 CPUs.',
  },
  {
    id: 'cpu',
    label: 'CPU',
    csvRow: { workload: 'cpu', cpus: '1', metric_unit: 'M records/s' },
    shortUnit: 'M',
    better: 'higher',
    measure: 'records scored per second',
    setup: 'A CPU-bound scoring loop on 1 CPU.',
  },
  {
    id: 'recovery',
    label: 'Recovery',
    csvRow: { workload: 'recovery', cpus: '4', metric_unit: 'µs' },
    better: 'lower',
    measure: 'to recover from a worker failure',
    setup: 'From the failure until the supervised replacement answers, on 4 CPUs.',
    note: 'Mesh panics. Elixir requests a supervised shutdown. Go and Rust signal the failure explicitly.',
  },
  {
    id: 'http',
    label: 'HTTP',
    csvRow: { workload: 'http', cpus: '4', offered_rps: '400', metric_unit: 'ms' },
    better: 'lower',
    measure: 'p99 response time',
    setup: '400 requests per second on 4 CPUs, one connection per request.',
  },
]

const FIGURE = new Intl.NumberFormat('en', { minimumSignificantDigits: 3, maximumSignificantDigits: 3 })

const fail = (message: string): never => { throw new Error(`benchmarks: ${message}`) }

function parseRows(csv: string): Row[] {
  const [head, ...lines] = csv.trim().split(/\r?\n/)
  const names = head.split(',')
  const at = COLUMNS.map((column) => {
    const i = names.indexOf(column)
    return i < 0 ? fail(`the CSV has no "${column}" column`) : i
  })
  return lines.map((line, i) => {
    const cells = line.split(',')
    if (cells.length !== names.length) fail(`CSV line ${i + 2} has ${cells.length} fields and the header has ${names.length}`)
    return Object.fromEntries(COLUMNS.map((column, j) => [column, cells[at[j]]])) as Row
  })
}

function medians(rows: Row[], id: string, key: RowKey): number[] {
  const where = Object.entries(key)
  const described = where.map(([column, value]) => `${column}=${value}`).join(', ')
  const selected = rows.filter((row) => where.every(([column, value]) => row[column as keyof RowKey] === value))
  if (selected.length === 0) fail(`panel "${id}" selects no CSV row (${described})`)
  return RUNTIMES.map((runtime) => {
    const found = selected.filter((row) => row.runtime === runtime)
    if (found.length !== 1) fail(`panel "${id}" needs one "${runtime}" row for ${described}, found ${found.length}`)
    const median = Number(found[0].median)
    if (!Number.isFinite(median) || median <= 0) fail(`panel "${id}", runtime "${runtime}": median "${found[0].median}" is not a positive finite number`)
    return median
  })
}

export function benchmarkPanels(csv: string): BenchmarkPanel[] {
  const rows = parseRows(csv)
  return PANELS.map(({ csvRow, shortUnit, ...panel }) => {
    const values = medians(rows, panel.id, { ...csvRow, blocks: RUN.trials })
    const longest = Math.max(...values)
    return {
      ...panel,
      headline: FIGURE.format(values[RUNTIMES.indexOf(HIGHLIGHTED)]),
      unit: shortUnit ?? csvRow.metric_unit,
      rows: values.map((median, i) => ({
        runtime: RUNTIMES[i],
        figure: FIGURE.format(median),
        share: median / longest,
        highlighted: RUNTIMES[i] === HIGHLIGHTED,
      })),
    }
  })
}
