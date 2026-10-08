// Drives headless Chrome over CDP: `node capture.mjs stills 1.2 3.1 ...` writes PNGs to stills/,
// `node capture.mjs video <poster-t>` pipes every frame (frame 0 = poster) into ffmpeg.
import { spawn } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { setTimeout as sleep } from 'node:timers/promises'

const here = new URL('.', import.meta.url).pathname
const [mode, ...args] = process.argv.slice(2)
const FPS = 30, DUR = Number(process.env.DUR || 24)
const PAGE = process.env.PAGE || 'video.html'
const port = 9400 + Math.floor(Math.random() * 400)

const chrome = spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', [
  '--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${here}chrome-profile-${port}`,
  '--window-size=1920,1080', '--hide-scrollbars', '--force-device-scale-factor=1', '--allow-file-access-from-files',
  '--no-first-run', '--no-default-browser-check', 'about:blank',
], { stdio: 'ignore' })

let targets
for (let i = 0; i < 100; i++) {
  try { targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json(); break } catch { await sleep(100) }
}
const page = targets.find((t) => t.type === 'page')
const ws = new WebSocket(page.webSocketDebuggerUrl)
await new Promise((r) => ws.addEventListener('open', r, { once: true }))
let id = 0
const pending = new Map()
ws.addEventListener('message', (e) => {
  const m = JSON.parse(e.data)
  if (m.id && pending.has(m.id)) { const { res, rej } = pending.get(m.id); pending.delete(m.id); m.error ? rej(new Error(m.error.message)) : res(m.result) }
})
const send = (method, params = {}) => new Promise((res, rej) => { const i = ++id; pending.set(i, { res, rej }); ws.send(JSON.stringify({ id: i, method, params })) })
const evaluate = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true })
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text)
  return r.result.value
}

await send('Page.enable')
await send('Emulation.setDeviceMetricsOverride', { width: 1920, height: 1080, deviceScaleFactor: 1, mobile: false })
await send('Page.navigate', { url: `file://${here}${PAGE}?capture` })
for (let i = 0; i < 200; i++) { if (await evaluate('!!window.ready').catch(() => false)) break; await sleep(50) }
await evaluate('window.ready')

const frame = async (t) => {
  await evaluate(`render(${t}); new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))`)
  const { data } = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false })
  return Buffer.from(data, 'base64')
}

if (mode === 'page') {
  // node capture.mjs page styleframes.html A1 A2 ...: one screenshot per ?f= value
  const [file, ...names] = args
  mkdirSync(`${here}stills`, { recursive: true })
  for (const n of names) {
    await send('Page.navigate', { url: `file://${here}${file}?capture&f=${n}` })
    await sleep(300)
    for (let i = 0; i < 200; i++) { if (await evaluate('!!window.ready').catch(() => false)) break; await sleep(50) }
    await evaluate('window.ready')
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))')
    const { data } = await send('Page.captureScreenshot', { format: 'png' })
    writeFileSync(`${here}stills/${n}.png`, Buffer.from(data, 'base64'))
  }
} else if (mode === 'stills') {
  mkdirSync(`${here}stills`, { recursive: true })
  for (const a of args) writeFileSync(`${here}stills/${process.env.TAG || 't'}${Number(a).toFixed(2)}.png`, await frame(Number(a)))
} else if (mode === 'events') {
  writeFileSync(args[0], JSON.stringify(await evaluate('window.EVENTS')))
} else if (mode === 'timing') {
  console.log(JSON.stringify(await evaluate('({ type: TYPE_TIMES, tabs: TAB_TIMES, cmd: CMD_TIMES })')))
} else if (mode === 'video') {
  const poster = Number(args[0])
  const out = args[1] || `${here}video-only.mp4`
  const ff = spawn('ffmpeg', ['-y', '-hide_banner', '-loglevel', 'error', '-f', 'image2pipe', '-framerate', String(FPS), '-i', '-',
    '-c:v', 'libx264', '-preset', 'slow', '-crf', '15', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', out], { stdio: ['pipe', 'inherit', 'inherit'] })
  const total = FPS * DUR
  // FRAMES=a:b renders only that range (for splicing a fix into a finished render)
  const [fa, fb] = (process.env.FRAMES || `0:${total}`).split(':').map(Number)
  for (let f = fa; f < fb; f++) {
    // frame 0 carries the poster so every platform's thumbnail shows it
    const png = await frame(f === 0 ? poster : f / FPS)
    if (!ff.stdin.write(png)) await new Promise((r) => ff.stdin.once('drain', r))
    if (f % 60 === 0) process.stdout.write(`${f}/${total}\n`)
  }
  ff.stdin.end()
  await new Promise((r) => ff.on('close', r))
}

ws.close()
chrome.kill()
await sleep(500)
try { rmSync(`${here}chrome-profile-${port}`, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 }) } catch {}
