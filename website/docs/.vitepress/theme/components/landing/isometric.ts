// The clustered-request figure, drawn at build time as isometric SVG.
// Faces and lines carry classes rather than colours so landing.css can paint
// them for either theme. The three `.hop` paths are the request's route,
// in order; ClusterFigure.vue moves the pulse along them.

type Pt = [number, number]
type Node = { x: number; y: number }

export interface Callout {
  at: 'ingress' | 'execution' | 'replica'
  title: string
  header: string
}

const C = Math.cos(Math.PI / 6)
const S = 0.5
const P = (x: number, y: number, z: number): Pt => [(x - y) * C, (x + y) * S - z]
const pts = (list: Pt[]) => list.map((p) => `${p[0].toFixed(2)},${p[1].toFixed(2)}`).join(' ')
const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')

interface Faces { top?: string; left?: string; right?: string; edge?: string }

function scene(blurId: string) {
  const items: string[] = []
  const all: Pt[] = []
  const track = (list: Pt[]) => { all.push(...list); return list }
  return {
    all,
    shadow(x: number, y: number, w: number, d: number, spread = 10, alpha = 0.16) {
      const f = [P(x + spread * 0.6, y + spread, 0), P(x + w + spread, y + spread * 0.6, 0), P(x + w + spread, y + d + spread, 0), P(x + spread * 0.6, y + d + spread, 0)]
      items.push(`<polygon points="${pts(f)}" class="shade" style="--a:${alpha}" filter="url(#${blurId})"/>`)
    },
    box(x: number, y: number, z: number, w: number, d: number, h: number, faces: Faces = {}) {
      const top = track([P(x, y, z + h), P(x + w, y, z + h), P(x + w, y + d, z + h), P(x, y + d, z + h)])
      const left = track([P(x, y + d, z), P(x + w, y + d, z), P(x + w, y + d, z + h), P(x, y + d, z + h)])
      const right = track([P(x + w, y, z), P(x + w, y + d, z), P(x + w, y + d, z + h), P(x + w, y, z + h)])
      items.push(`<g class="${faces.edge ?? 'edge'}" stroke-width="1" stroke-linejoin="round">`
        + `<polygon points="${pts(left)}" class="${faces.left ?? 'f-left'}"/>`
        + `<polygon points="${pts(right)}" class="${faces.right ?? 'f-right'}"/>`
        + `<polygon points="${pts(top)}" class="${faces.top ?? 'f-top'}"/></g>`)
    },
    flat(x: number, y: number, z: number, w: number, d: number, cls: string) {
      items.push(`<polygon points="${pts(track([P(x, y, z), P(x + w, y, z), P(x + w, y + d, z), P(x, y + d, z)]))}" class="${cls}"/>`)
    },
    line(list: [number, number, number][], cls: string, width = 1, dash?: string) {
      const p = track(list.map((q) => P(...q)))
      items.push(`<polyline points="${pts(p)}" fill="none" class="${cls}" stroke-width="${width}"${dash ? ` stroke-dasharray="${dash}"` : ''} stroke-linecap="round" stroke-linejoin="round"/>`)
    },
    planeText(x: number, y: number, z: number, text: string, size = 9) {
      const p = P(x, y, z)
      items.push(`<text transform="matrix(${C},${S},0,1,${p[0].toFixed(2)},${p[1].toFixed(2)})" font-size="${size}" class="plane">${esc(text)}</text>`)
    },
    svg(cls: string, after: string, pad: number, blur: number) {
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity
      for (const [x, y] of all) { minX = Math.min(minX, x); minY = Math.min(minY, y); maxX = Math.max(maxX, x); maxY = Math.max(maxY, y) }
      const vb = [minX - pad, minY - pad, maxX - minX + pad * 2, maxY - minY + pad * 2].map((v) => v.toFixed(1)).join(' ')
      return `<svg class="${cls}" viewBox="${vb}" xmlns="http://www.w3.org/2000/svg" aria-hidden="true" preserveAspectRatio="xMidYMid meet">`
        + `<defs><filter id="${blurId}" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="${blur}"/></filter></defs>`
        + items.join('') + after + '</svg>'
    },
  }
}

const PW = 190, PD = 190, PH = 14

export function clusterFigure(callouts: Callout[], size: 'lg' | 'sm'): string {
  const s = scene(`iso-blur-${size}`)
  const nodes: Record<Callout['at'], Node> = { ingress: { x: 0, y: 250 }, execution: { x: 250, y: 0 }, replica: { x: 0, y: 0 } }
  const { ingress: ig, execution: ex, replica: rp } = nodes
  for (const n of [rp, ig, ex]) s.shadow(n.x, n.y, PW, PD, 20, 0.17)
  for (const n of [rp, ig, ex]) {
    s.box(n.x, n.y, 0, PW, PD, PH)
    for (let k = 1; k < 4; k++) s.line([[n.x + k * PW / 4, n.y, PH + 0.2], [n.x + k * PW / 4, n.y + PD, PH + 0.2]], 'grid')
    for (let k = 1; k < 4; k++) s.line([[n.x, n.y + k * PD / 4, PH + 0.2], [n.x + PW, n.y + k * PD / 4, PH + 0.2]], 'grid')
  }
  const cx = (n: Node) => n.x + PW / 2
  const cy = (n: Node) => n.y + PD / 2
  const Z = PH + 0.5
  s.line([[cx(ig), ig.y + PD + 110, Z], [cx(ig), cy(ig), Z]], 'wire hop', 1.4)
  s.line([[cx(ig), cy(ig), Z], [cx(ig) + 150, cy(ig), Z], [cx(ex), cy(ex) + 60, Z], [cx(ex), cy(ex), Z]], 'wire-hot hop', 2)
  s.line([[cx(ex), cy(ex), Z], [cx(rp) + 40, cy(rp), Z]], 'wire-dash hop', 1.3, '4 5')

  s.shadow(cx(ig) - 32, cy(ig) - 26, 64, 52, 6, 0.14)
  s.box(cx(ig) - 32, cy(ig) - 26, PH, 64, 52, 30)
  s.box(cx(ig) - 32, cy(ig) - 26, PH + 30, 64, 52, 6, { top: 'f-cap' })
  s.flat(cx(ig) - 22, cy(ig) - 16, PH + 36.2, 30, 4, 'f-bar')
  s.flat(cx(ig) - 22, cy(ig) - 6, PH + 36.2, 44, 4, 'f-bar-2')

  // The front face is wide enough for the handler name (12 mono glyphs, about 65 units at size 9).
  s.shadow(cx(ex) - 42, cy(ex) - 32, 84, 64, 6, 0.16)
  s.box(cx(ex) - 42, cy(ex) - 32, PH, 84, 64, 44, { top: 'f-hot', left: 'f-hot-l', right: 'f-hot-r', edge: 'edge-hot' })
  s.planeText(cx(ex) - 32.5, cy(ex) + 32, PH + 17, 'show_account')

  for (let k = 0; k < 3; k++) {
    if (k === 0) s.shadow(cx(rp) - 34, cy(rp) - 26, 68, 52, 5, 0.14)
    s.box(cx(rp) - 34, cy(rp) - 26, PH + 14 + k * 15, 68, 52, 4, k === 2 ? { top: 'f-cap' } : {})
  }
  const rz = PH + 14 + 30 + 4.2
  s.flat(cx(rp) - 26, cy(rp) - 18, rz, 40, 4, 'f-bar')
  s.flat(cx(rp) - 26, cy(rp) - 8, rz, 28, 4, 'f-bar-2')
  s.flat(cx(rp) - 26, cy(rp) + 2, rz, 48, 4, 'f-bar-2')
  s.flat(cx(rp) - 26, cy(rp) + 12, rz, 18, 4, 'f-hot')

  const pin = P(cx(ig), ig.y + PD + 110, Z)
  let after = `<circle cx="${pin[0].toFixed(1)}" cy="${pin[1].toFixed(1)}" r="3.2" class="dot"/>`
  const lift: Record<Callout['at'], { dz: number; side: 'l' | 'r'; rise: number }> = {
    ingress: { dz: 36, side: 'l', rise: 88 },
    execution: { dz: 44, side: 'r', rise: 88 },
    replica: { dz: 52, side: 'r', rise: 64 },
  }
  for (const c of callouts) {
    const { dz, side, rise } = lift[c.at]
    const n = nodes[c.at]
    const top = P(cx(n), cy(n), PH + dz)
    const ax = top[0], ay = top[1] - 6, yTop = ay - rise
    const tx = side === 'l' ? ax - 12 : ax + 12
    const anchor = side === 'l' ? 'end' : 'start'
    s.all.push([tx + (side === 'l' ? -230 : 230), yTop - 4])
    after += `<g><line x1="${ax.toFixed(1)}" y1="${ay.toFixed(1)}" x2="${ax.toFixed(1)}" y2="${(yTop - 2).toFixed(1)}" class="wire" stroke-width="1"/>`
      + `<circle cx="${ax.toFixed(1)}" cy="${ay.toFixed(1)}" r="3.2" class="dot"/>`
      + `<text x="${tx.toFixed(1)}" y="${(yTop + 12).toFixed(1)}" text-anchor="${anchor}" class="lbl-b">${esc(c.title)}</text>`
      + `<text x="${tx.toFixed(1)}" y="${(yTop + 31).toFixed(1)}" text-anchor="${anchor}" class="lbl-c">${esc(c.header)}</text></g>`
  }
  if (callouts.length) {
    after += `<text x="${(pin[0] - 12).toFixed(1)}" y="${(pin[1] + 4).toFixed(1)}" text-anchor="end" class="lbl">GET /accounts/7</text>`
    s.all.push([pin[0] - 150, pin[1] + 12])
  }
  after += '<circle class="pulse" r="5" stroke-width="2" opacity="0"/>'
  return s.svg(`fig-${size}`, after, callouts.length ? 8 : 4, 12)
}
