// Renders og-image.html to docs/public/og-image-v3.png (1200x630) with a local
// Chrome or Chromium: CHROME_PATH if set, else the first one found on PATH, in
// the usual macOS app locations, or in the Playwright browser cache.
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const site = path.resolve(here, '..')
const out = path.join(site, 'docs/public/og-image-v3.png')

const PATH_NAMES = ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser', 'chrome']
const MAC_APPS = ['Google Chrome.app/Contents/MacOS/Google Chrome', 'Chromium.app/Contents/MacOS/Chromium']
// Inside <cache>/chromium[_headless_shell]-<revision>/chrome-<platform>-<arch>/
const PLAYWRIGHT_BINARIES = [
  'chrome-headless-shell', 'chrome-headless-shell.exe', 'chrome', 'chrome.exe',
  'Chromium.app/Contents/MacOS/Chromium', 'Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing',
]

const list = (dir) => { try { return fs.readdirSync(dir) } catch { return [] } }
const playwrightCaches = () => [
  process.env.PLAYWRIGHT_BROWSERS_PATH,
  path.join(os.homedir(), 'Library/Caches/ms-playwright'),
  path.join(os.homedir(), '.cache/ms-playwright'),
  process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, 'ms-playwright'),
].filter(Boolean)

function* candidates() {
  const dirs = (process.env.PATH ?? '').split(path.delimiter).filter(Boolean)
  for (const name of PATH_NAMES) yield* dirs.map((dir) => path.join(dir, name))
  for (const apps of ['/Applications', path.join(os.homedir(), 'Applications')]) yield* MAC_APPS.map((app) => path.join(apps, app))
  for (const cache of playwrightCaches()) {
    const revisions = list(cache)
      .filter((name) => /^chromium(?:_headless_shell)?-\d+$/.test(name))
      .sort((a, b) => Number(b.split('-')[1]) - Number(a.split('-')[1]))
    for (const revision of revisions) {
      for (const platform of list(path.join(cache, revision)).filter((name) => name.startsWith('chrome-'))) {
        yield* PLAYWRIGHT_BINARIES.map((binary) => path.join(cache, revision, platform, binary))
      }
    }
  }
}

function findChrome() {
  const chosen = process.env.CHROME_PATH
  if (chosen) {
    if (!fs.existsSync(chosen)) throw new Error(`CHROME_PATH is set, but there is no file at ${chosen}`)
    return chosen
  }
  const tried = []
  for (const candidate of candidates()) {
    if (fs.existsSync(candidate)) return candidate
    tried.push(candidate)
  }
  tried.push(...playwrightCaches().map((cache) => path.join(cache, 'chromium*-<revision>', 'chrome-*')))
  throw new Error(`no Chrome or Chromium found; set CHROME_PATH. Tried:\n  ${tried.join('\n  ')}`)
}

const chrome = findChrome()
console.log(`using ${chrome}`)

const geist = fs.readFileSync(path.join(site, 'node_modules/@fontsource-variable/geist/files/geist-latin-wght-normal.woff2')).toString('base64')
const lockup = fs.readFileSync(path.join(site, 'docs/.vitepress/theme/assets/mesh-lockup.svg'), 'utf8')
const html = fs.readFileSync(path.join(here, 'og-image.html'), 'utf8').replace('{{GEIST}}', geist).replace('{{LOCKUP}}', lockup)

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mesh-og-'))
try {
  const page = path.join(dir, 'card.html')
  fs.writeFileSync(page, html)
  execFileSync(chrome, [
    '--headless', '--no-sandbox', '--hide-scrollbars', '--force-device-scale-factor=1',
    '--window-size=1200,630', '--virtual-time-budget=3000',
    `--user-data-dir=${path.join(dir, 'profile')}`, `--screenshot=${out}`, pathToFileURL(page).href,
  ], { stdio: 'ignore' })
} finally {
  fs.rmSync(dir, { recursive: true, force: true })
}
console.log(`wrote ${path.relative(process.cwd(), out)}`)
