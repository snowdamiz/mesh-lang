import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { transformWithEsbuild } from 'vite'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const modulePath = path.join(root, 'website/docs/.vitepress/theme/components/landing/jobs.ts')
const { code } = await transformWithEsbuild(fs.readFileSync(modulePath, 'utf8'), modulePath, { loader: 'ts' })
const { jobsPanels, JOBS_RUN } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)

const published = fs.readFileSync(path.join(root, JOBS_RUN.dir, 'comparison.csv'), 'utf8')

test('the published run charts four engines on each of four workloads', () => {
  const panels = jobsPanels(published)
  assert.deepEqual(panels.map((panel) => panel.id), ['short', 'dense', 'large', 'slow'])
  for (const panel of panels) {
    assert.deepEqual(panel.rows.map((row) => row.runtime), ['Mesh', 'Oban', 'River', 'Apalis'], panel.id)
    assert.equal(Math.max(...panel.rows.map((row) => row.share)), 1, panel.id)
    assert.match(panel.measure, /^\d+\.\d× faster than Oban$/, panel.id)
  }
})

test('the page refuses to build once Mesh stops beating Oban', () => {
  const lines = published.trim().split('\n')
  const slower = lines.map((line) => (line.startsWith('slow,') && line.includes(',Mesh,') ? line.replace(/,Mesh,[\d.]+,/, ',Mesh,9999,') : line)).join('\n')
  assert.throws(() => jobsPanels(slower), /no longer beats Oban/)
})

test('a missing engine row names the workload', () => {
  const without = published.split('\n').filter((line) => !line.startsWith('dense,1024,5,River')).join('\n')
  assert.throws(() => jobsPanels(without), /workload "dense" needs one "River" row, found 0/)
})
