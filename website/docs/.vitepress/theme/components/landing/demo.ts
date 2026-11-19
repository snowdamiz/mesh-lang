// The product-window walkthrough as data plus a pure state machine.
// landing.data.mts declares each step as a list of actions. compileStep turns
// that list into a timeline of patches over a Frame, and demoReducer moves the
// walkthrough between steps and phases. The component only owns the clock.

export type StepId = 'write' | 'check' | 'build' | 'cluster'
export type Phase = 'typing' | 'showing' | 'paused'

export interface Seg { cls?: string; text: string }
export interface TermRow { segs: Seg[]; diag?: boolean; fade?: boolean }
export interface Span { line: number; from: number; to: number }
export interface Focus { line: number; mode: 'caret' | 'center' }

/** A visible editor line: the first `len` characters of the line keyed `key`. */
export interface LineRef { key: string; len: number }

export interface Frame {
  lines: LineRef[]
  caret: { line: number; col: number } | null
  sel: Span | null
  sq: Span | null
  hl: number[]
  err: number | null
  focus: Focus | null
  problems: number
  rows: TermRow[]
  termCaret: boolean
}

export type EditorPatch = Partial<Pick<Frame, 'caret' | 'sq' | 'hl' | 'err' | 'focus' | 'problems'>>

export type Action =
  | { do: 'wait'; ms: number }
  | { do: 'command'; text: string; ms: number }
  | { do: 'prompt'; caret?: boolean }
  | { do: 'print'; rows: TermRow[]; ms?: number }
  | { do: 'type-file'; caret: { line: number; col: number } }
  | { do: 'replace'; line: number; from: number; to: number; text: string }
  | { do: 'editor'; patch: EditorPatch }
  | { do: 'term-caret'; on: boolean }
  | { do: 'fade' }
  /** marks the step's still frame: what it shows when it does not play */
  | { do: 'still' }

export interface StepStart extends EditorPatch {
  /** start from an empty editor instead of the whole file */
  empty?: boolean
  termCaret?: boolean
}

export interface Step {
  id: StepId
  label: string
  /** caption; <b> is the only markup */
  caption: string
  cwd: string
  file: 'counter' | 'cluster'
  /** time the end state stays on screen before the next step */
  hold: number
  start: StepStart
  actions: Action[]
}

export interface Timeline {
  init: Frame
  events: { at: number; apply: (f: Frame) => void }[]
  /** time of the frame shown under reduced motion and when picked while paused; the end unless marked */
  still: number
  end: number
  duration: number
  /** every line text this step can show, so the loader can highlight it */
  keys: string[]
}

const PROMPT: Seg[] = [{ cls: 'ps', text: '$' }, { text: ' ' }]
const SELECT_MS = 420
const REPLACE_CHAR_MS = 55
const FILE_CHUNK = 3
const FILE_CHUNK_MS = 26
const FILE_NEWLINE_MS = 70

export function compileStep(step: Step, source: string): Timeline {
  const fileLines = source.split('\n')
  const text = fileLines.slice()
  const keys = new Set(fileLines)
  const full = (): LineRef[] => text.map((key) => ({ key, len: key.length }))
  const { empty, termCaret = false, ...patch } = step.start
  const init: Frame = {
    lines: empty ? [{ key: fileLines[0], len: 0 }] : full(),
    caret: null, sel: null, sq: null, hl: [], err: null, focus: null, problems: 0,
    rows: [{ segs: [...PROMPT] }],
    termCaret,
    ...patch,
  }

  const events: Timeline['events'] = []
  let at = 0
  let still: number | undefined
  const at_ = (apply: (f: Frame) => void) => events.push({ at, apply })
  const lastRow = (f: Frame) => f.rows[f.rows.length - 1]

  for (const action of step.actions) {
    switch (action.do) {
      case 'wait':
        at += action.ms
        break
      case 'command':
        at_((f) => { f.termCaret = true })
        for (let k = 1; k <= action.text.length; k++) {
          const typed = action.text.slice(0, k)
          at_((f) => { lastRow(f).segs = [...PROMPT, { text: typed }] })
          at += action.ms
        }
        at_((f) => { f.termCaret = false })
        break
      case 'prompt':
        at_((f) => { f.rows.push({ segs: [...PROMPT] }); f.termCaret = !!action.caret })
        break
      case 'print':
        for (const row of action.rows) {
          at_((f) => { f.rows.push(row) })
          at += action.ms ?? 0
        }
        break
      case 'type-file': {
        let n = 0
        while (n < source.length) {
          if (source[n] === '\n') {
            n++
            while (source[n] === ' ') n++
            at += FILE_NEWLINE_MS
          } else {
            n = Math.min(source.length, n + FILE_CHUNK)
            at += FILE_CHUNK_MS
          }
          const typed = source.slice(0, n).split('\n')
          at_((f) => {
            f.lines = typed.map((t, i) => ({ key: fileLines[i], len: t.length }))
            const line = typed.length - 1
            f.caret = { line, col: typed[line].length }
            f.focus = { line, mode: 'caret' }
          })
        }
        const { caret } = action
        at_((f) => { f.caret = caret })
        break
      }
      case 'replace': {
        const { line, from, to } = action
        const before = text[line].slice(0, from)
        const after = text[line].slice(to)
        at_((f) => { f.sel = { line, from, to }; f.caret = { line, col: to } })
        at += SELECT_MS
        for (let k = 0; k <= action.text.length; k++) {
          const key = before + action.text.slice(0, k) + after
          keys.add(key)
          const col = from + k
          at_((f) => { f.sel = null; f.lines[line] = { key, len: key.length }; f.caret = { line, col } })
          if (k < action.text.length) at += REPLACE_CHAR_MS
        }
        text[line] = before + action.text + after
        break
      }
      case 'editor': {
        const { patch: p } = action
        at_((f) => { Object.assign(f, p) })
        break
      }
      case 'term-caret': {
        const { on } = action
        at_((f) => { f.termCaret = on })
        break
      }
      case 'fade':
        at_((f) => { f.rows = f.rows.map((r) => ({ ...r, fade: true })) })
        break
      case 'still':
        still = at
        break
    }
  }
  return { init, events, still: still ?? at, end: at, duration: at + step.hold, keys: [...keys] }
}

export function cloneFrame(f: Frame): Frame {
  return { ...f, lines: f.lines.slice(), hl: f.hl.slice(), rows: f.rows.map((r) => ({ ...r, segs: r.segs.slice() })) }
}

export interface DemoState { step: number; t: number; phase: Phase }

export type DemoEvent =
  | { type: 'tick'; dt: number }
  | { type: 'select'; step: number }
  | { type: 'toggle' }

export interface Timing { still: number; end: number; duration: number }

/**
 * typing: the step's script is playing. showing: its end state is on screen
 * until the step's duration runs out. paused: the clock is held, and a step
 * picked while paused opens at its still frame. Under reduced motion nothing
 * plays: each step opens at its still frame and holds it for the step's hold.
 */
export function demoReducer(s: DemoState, e: DemoEvent, timing: Timing[], reduced: boolean): DemoState {
  const phaseAt = (step: number, t: number): Phase => (!reduced && t < timing[step].end ? 'typing' : 'showing')
  const open = (step: number): DemoState => {
    const t = reduced ? timing[step].still : 0
    return { step, t, phase: phaseAt(step, t) }
  }
  const last = (step: number) => (reduced ? timing[step].still + timing[step].duration - timing[step].end : timing[step].duration)
  switch (e.type) {
    case 'tick': {
      if (s.phase === 'paused') return s
      const t = s.t + e.dt
      if (t >= last(s.step)) return open((s.step + 1) % timing.length)
      return { ...s, t, phase: phaseAt(s.step, t) }
    }
    case 'select':
      return s.phase === 'paused' ? { step: e.step, t: timing[e.step].still, phase: 'paused' } : open(e.step)
    case 'toggle':
      return s.phase === 'paused' ? { ...s, phase: phaseAt(s.step, s.t) } : { ...s, phase: 'paused' }
  }
}
