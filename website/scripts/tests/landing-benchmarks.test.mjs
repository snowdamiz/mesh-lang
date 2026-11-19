import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { transformWithEsbuild } from 'vite'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const modulePath = path.join(root, 'website/docs/.vitepress/theme/components/landing/benchmarks.ts')
const { code } = await transformWithEsbuild(fs.readFileSync(modulePath, 'utf8'), modulePath, { loader: 'ts' })
const { benchmarkPanels, RUN } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)

const published = fs.readFileSync(path.join(root, RUN.dir, 'comparison.csv'), 'utf8')
const GO_ACTORS = 'actor,4,64,0,M round trips/s,Go,'
const COLUMN = { unit: 4, runtime: 5, blocks: 6, median: 7 }

function withRow(startsWith, replace) {
  const lines = published.split('\n')
  const at = lines.findIndex((line) => line.startsWith(startsWith))
  assert.notEqual(at, -1, `the published CSV has a row starting with ${startsWith}`)
  lines.splice(at, 1, ...replace(lines[at]))
  return lines.join('\n')
}

function withCell(line, column, value) {
  const cells = line.split(',')
  cells[column] = value
  return cells.join(',')
}

const goActors = (csv) => benchmarkPanels(csv).find((panel) => panel.id === 'actors').rows.find((row) => row.runtime === 'Go')

test('the published run charts four runtimes on each of four workloads', () => {
  const panels = benchmarkPanels(published)
  assert.deepEqual(panels.map((panel) => panel.id), ['actors', 'cpu', 'recovery', 'http'])
  for (const panel of panels) {
    assert.deepEqual(panel.rows.map((row) => row.runtime), ['Mesh', 'Rust/Tokio', 'Go', 'Elixir'], panel.id)
    assert.deepEqual(panel.rows.filter((row) => row.highlighted).map((row) => row.runtime), ['Mesh'], panel.id)
    assert.equal(Math.max(...panel.rows.map((row) => row.share)), 1, `${panel.id}: the longest bar fills the track`)
    for (const row of panel.rows) assert.match(row.figure, /^[\d.,]+$/, `${panel.id} ${row.runtime}`)
  }
})

test('a figure is the median at three significant figures', () => {
  const median = (value) => withRow(GO_ACTORS, (line) => [withCell(line, COLUMN.median, value)])
  assert.equal(goActors(median('1.2039')).figure, '1.20')
  assert.equal(goActors(median('0.78288')).figure, '0.783')
  assert.equal(goActors(median('1234.5')).figure, '1,230')
})

test('a bar is the median as a share of the longest in its panel', () => {
  const csv = withRow(GO_ACTORS, (line) => [withCell(line, COLUMN.median, '1000')])
  const rows = benchmarkPanels(csv).find((panel) => panel.id === 'actors').rows
  const mesh = Number(published.split('\n').find((line) => line.startsWith('actor,4,64,0,M round trips/s,Mesh,')).split(',')[COLUMN.median])
  assert.equal(rows.find((row) => row.runtime === 'Go').share, 1)
  assert.equal(rows.find((row) => row.runtime === 'Mesh').share, mesh / 1000)
})

const BROKEN = [
  ['a selected row is missing', withRow(GO_ACTORS, () => []), /needs one "Go" row.*found 0/],
  ['a selected row appears twice', withRow(GO_ACTORS, (line) => [line, line]), /needs one "Go" row.*found 2/],
  ['a median is empty', withRow(GO_ACTORS, (line) => [withCell(line, COLUMN.median, '')]), /not a positive finite number/],
  ['a median is not a number', withRow(GO_ACTORS, (line) => [withCell(line, COLUMN.median, 'n/a')]), /not a positive finite number/],
  ['the unit differs from the one the page prints', withRow(GO_ACTORS, (line) => [withCell(line, COLUMN.unit, 'K round trips/s')]), /needs one "Go" row.*found 0/],
  ['the trial count differs from the footnote', withRow(GO_ACTORS, (line) => [withCell(line, COLUMN.blocks, '3')]), /needs one "Go" row.*found 0/],
  ['a column is missing', published.replace('median', 'mean'), /no "median" column/],
  ['a field is quoted', withRow(GO_ACTORS, (line) => [withCell(line, COLUMN.runtime, '"Go"')]), /quoted field/],
]

for (const [name, csv, message] of BROKEN) {
  test(`the build fails when ${name}`, () => {
    assert.throws(() => benchmarkPanels(csv), message)
  })
}
