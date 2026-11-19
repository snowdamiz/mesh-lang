// Everything the landing page shows, as data. Every Mesh sample is highlighted
// here at build time with the grammar and themes the docs use, so the page
// ships no highlighter.
import { defineLoader } from 'vitepress'
import { createHighlighter, type HighlighterCore } from 'shiki'
import meshGrammar from '../../../../../../tools/editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'
import meshLight from '../../shiki/mesh-light.json'
import meshDark from '../../shiki/mesh-dark.json'
import { compileStep, type Seg, type Step, type TermRow } from './demo'
import { clusterFigure, type Callout } from './isometric'

const COUNTER = `type Message do
  Add(Int)
  Get(Pid<Int>)
end

actor counter(total :: Int) do
  receive do
    Add(n) -> counter(total + n)
    Get(reply) -> do
      send(reply, total)
      counter(total)
    end
  end
end

actor client(pid :: Pid<Message>) do
  send(pid, Add(2))
  send(pid, Add(40))
  send(pid, Get(self()))
  receive do
    total -> println("total = #{total}")
  end
end

fn main() do
  let pid = spawn(counter, 0)
  spawn(client, pid)
  Timer.sleep(50)
end`

const CLUSTER = `@cluster(3)
pub fn rebuild_index() -> Int do
  42
end

pub fn show_account(request :: Request) -> Response do
  case Request.param(request, "id") do
    Some(id) -> HTTP.response(200, json { id: id, owner: "ada" })
    None -> HTTP.response(400, json { error: "missing id" })
  end
end

pub fn open_account(request :: Request) -> Response do
  HTTP.response(201, Request.body(request))
end

fn main() do
  let router = HTTP.router()
    |> HTTP.on_get("/accounts/:id", HTTP.clustered(2, show_account))
    |> HTTP.on_post("/accounts", HTTP.clustered(3, open_account))
  HTTP.serve(router, 8080)
end`

const PIPES = `fn fib(0) = 0
fn fib(1) = 1
fn fib(n) = fib(n - 1) + fib(n - 2)

fn main() do
  let total = [1, 2, 3, 4, 5, 6]
    |> List.map(fn x -> x * x end)
    |> List.filter(fn x -> x > 10 end)
    |> List.reduce(0, fn acc, x -> acc + x end)
  println("#{total}")
end`

const ROUTER = `fn main() do
  let router = HTTP.router()
    |> HTTP.on_get("/accounts/:id", HTTP.clustered(2, show_account))
    |> HTTP.on_post("/accounts", HTTP.clustered(3, open_account))
  HTTP.serve(router, 8080)
end`

const STDLIB = `struct Event do
  id :: String
  name :: String
  at :: String
end deriving(Json)

fn store(body :: String) -> Int!String do
  let db = Sqlite.open("events.db")?
  Sqlite.execute(db, "INSERT INTO events (body) VALUES (?)", [body])
end

pub fn track(request :: Request) -> Response do
  case Request.param(request, "name") do
    Some(name) when Regex.is_match(~r/^[a-z_]+$/, name) ->
      let at = DateTime.to_iso8601(DateTime.utc_now())
      let event = Event { id: Crypto.uuid4(), name: name, at: at }
      let body = Json.encode(event)
      Job.async(fn() -> store(body) end)
      HTTP.response(202, json { id: event.id, node: Node.self() })
    _ -> HTTP.response(400, json { error: "bad event name" })
  end
end

fn main() do
  let router = HTTP.router()
    |> HTTP.on_post("/events/:name", track)
  HTTP.serve(router, Env.get_int("PORT", 8080))
end`

const WRONG_SEND = `send(pid, "forty")`

const SEND_LINE = 17
const row = (...segs: Seg[]): TermRow => ({ segs })
const diag = (...segs: Seg[]): TermRow => ({ segs, diag: true })

const STEPS: Step[] = [
  {
    id: 'write',
    label: 'Write',
    caption: 'Message types and actors are part of the syntax. <b>Pid&lt;Message&gt;</b> is a type.',
    cwd: '~/hello_mesh',
    file: 'counter',
    hold: 1200,
    start: { empty: true, caret: { line: 0, col: 0 }, termCaret: true },
    actions: [
      { do: 'wait', ms: 300 },
      { do: 'command', text: 'meshc init hello_mesh', ms: 26 },
      { do: 'prompt' },
      { do: 'command', text: 'cd hello_mesh', ms: 26 },
      { do: 'prompt', caret: true },
      { do: 'wait', ms: 250 },
      { do: 'type-file', caret: { line: 28, col: 3 } },
    ],
  },
  {
    id: 'check',
    label: 'Check',
    caption: 'Send a <b>Pid&lt;Message&gt;</b> a String and the build stops with E0014.',
    cwd: '~/hello_mesh',
    file: 'counter',
    hold: 1200,
    start: { hl: [SEND_LINE], caret: { line: SEND_LINE, col: 20 }, focus: { line: SEND_LINE, mode: 'center' }, termCaret: true },
    actions: [
      { do: 'wait', ms: 500 },
      { do: 'replace', line: SEND_LINE, from: 12, to: 19, text: '"forty"' },
      { do: 'wait', ms: 350 },
      { do: 'command', text: 'meshc build .', ms: 30 },
      { do: 'wait', ms: 420 },
      { do: 'editor', patch: { err: SEND_LINE, sq: { line: SEND_LINE, from: 2, to: 20 }, problems: 1 } },
      {
        do: 'print',
        ms: 45,
        rows: [
          row({ cls: 'err', text: '[E0014] Error: message type mismatch: expected `Message`, found `String`' }),
          diag({ cls: 'box', text: '    ╭─[ ' }, { cls: 'dim', text: './main.mpl:18:3' }, { cls: 'box', text: ' ]' }),
          diag({ cls: 'box', text: '    │' }),
          diag({ cls: 'dim', text: ' 18' }, { cls: 'box', text: ' │ ' }, { cls: 'hdr', text: '  send(pid, "forty")' }),
          diag({ cls: 'box', text: '    │ ' }, { cls: 'err', text: '  ─────────┬────────' }),
          diag({ cls: 'box', text: '    │ ' }, { cls: 'err', text: '           ╰────────── expected Message, found String' }),
          diag({ cls: 'box', text: '    │' }),
          diag({ cls: 'box', text: '    │ ' }, { cls: 'hint', text: 'Help: this Pid accepts messages of type Message' }),
          diag({ cls: 'box', text: '────╯' }),
          row({ cls: 'err', text: 'error: Compilation failed due to errors above.' }),
        ],
      },
      { do: 'still' },
      { do: 'wait', ms: 3800 },
      { do: 'editor', patch: { sq: null } },
      { do: 'replace', line: SEND_LINE, from: 12, to: 19, text: 'Add(40)' },
      { do: 'editor', patch: { err: null, problems: 0 } },
      { do: 'fade' },
      { do: 'prompt', caret: true },
    ],
  },
  {
    id: 'build',
    label: 'Build',
    caption: '<b>meshc build .</b> compiles through LLVM to one native executable. No VM to install.',
    cwd: '~/hello_mesh',
    file: 'counter',
    hold: 1400,
    start: { focus: { line: 20, mode: 'center' } },
    actions: [
      { do: 'wait', ms: 300 },
      { do: 'command', text: 'meshc build .', ms: 30 },
      { do: 'wait', ms: 900 },
      { do: 'print', rows: [row({ cls: 'dim', text: '  ' }, { cls: 'ok', text: 'Compiled:' }, { text: ' ./output' })] },
      { do: 'wait', ms: 500 },
      { do: 'prompt' },
      { do: 'command', text: './output', ms: 45 },
      { do: 'wait', ms: 380 },
      { do: 'print', rows: [row({ cls: 'out', text: 'total = 42' })] },
      { do: 'editor', patch: { hl: [20] } },
      { do: 'prompt', caret: true },
    ],
  },
  {
    id: 'cluster',
    label: 'Cluster',
    caption: '<b>HTTP.clustered</b> lets the runtime pick the node for each request.',
    cwd: '~/hello_cluster',
    file: 'cluster',
    hold: 2400,
    start: {},
    actions: [
      { do: 'wait', ms: 500 },
      { do: 'editor', patch: { hl: [18], focus: { line: 18, mode: 'center' } } },
      { do: 'wait', ms: 600 },
      { do: 'command', text: 'curl -si localhost:8080/accounts/7 | grep X-Mesh', ms: 24 },
      { do: 'wait', ms: 700 },
      {
        do: 'print',
        ms: 140,
        rows: [
          row({ cls: 'hdr', text: 'X-Mesh-Ingress-Node:' }, { text: ' ' }, { cls: 'ph', text: '‹node›' }),
          row({ cls: 'hdr', text: 'X-Mesh-Execution-Node:' }, { text: ' ' }, { cls: 'ph', text: '‹node›' }),
          row({ cls: 'hdr', text: 'X-Mesh-Routed-Remotely:' }, { text: ' ' }, { cls: 'ph', text: '‹true | false›' }),
          row({ cls: 'hdr', text: 'X-Mesh-Continuity-Request-Key:' }, { text: ' ' }, { cls: 'ph', text: '‹key›' }),
        ],
      },
      { do: 'print', rows: [row({ text: ' ' })] },
      { do: 'prompt', caret: true },
    ],
  },
]

const FILES = { counter: COUNTER, cluster: CLUSTER }

const CALLOUTS: Callout[] = [
  { at: 'ingress', title: 'Ingress node', header: 'X-Mesh-Ingress-Node' },
  { at: 'execution', title: 'Execution node', header: 'X-Mesh-Execution-Node' },
  { at: 'replica', title: 'Continuity record', header: 'X-Mesh-Continuity-Request-Key' },
]

export type TileId = 'syntax' | 'supervisor' | 'pid' | 'binary' | 'safety'

/** [text, colour, italic] on the night palette, for the editor */
export type DarkToken = [string, string, 1?]

export interface Tile { id: TileId; span: 4 | 5 | 7; dark?: boolean; title: string; line: string }

export interface InstallOption { id: 'unix' | 'win'; label: string; command: string; note: string }

export interface LandingData {
  hero: { title: string[]; lead: string; cta: { text: string; href: string }; firstRun: string[] }
  install: InstallOption[]
  demo: { steps: Step[]; files: Record<Step['file'], string>; tokens: Record<string, DarkToken[]> }
  features: { title: [string, string]; tiles: Tile[] }
  art: {
    pipes: string
    supervisor: { root: string; workers: string[]; hint: string }
    pid: { code: string; message: string; src: string; label: string }
    binary: { label: string; rows: { command?: string; output?: string; ok?: string; dim?: boolean }[] }
    safety: { label: string; source: string; blocked: string[]; allowed: string[] }
  }
  distribution: { title: [string, string]; lead: string; link: { text: string; href: string }; figure: { lg: string; sm: string; label: string }; code: string }
  toolchain: { title: [string, string]; commands: { id: string; command: string; gloss: string }[]; note: string }
  stdlib: { title: [string, string]; file: string; lines: { html: string; modules: string[]; groups: string[] }[]; groups: { name: string; modules: string[] }[]; link: { text: string; href: string } }
  quote: { text: string; author: string; role: string }
  close: { title: string }
}

type Copy = Omit<LandingData, 'demo' | 'art' | 'distribution' | 'stdlib'>

const COPY: Copy = {
  hero: {
    title: ['Typed actors.', 'Native binaries.'],
    lead: 'Mesh is a statically typed, actor\u2011based language for native services and distributed systems.',
    cta: { text: 'Read the docs', href: '/docs/getting-started/' },
    firstRun: ['meshc init hello_mesh', 'cd hello_mesh', 'meshc build .', './output'],
  },
  install: [
    { id: 'unix', label: 'macOS & Linux', command: 'curl -sSf https://meshlang.dev/install.sh | sh', note: 'macOS and Linux (glibc), x86_64 and arm64.' },
    { id: 'win', label: 'Windows', command: 'irm https://meshlang.dev/install.ps1 | iex', note: 'Windows x86_64, in PowerShell.' },
  ],
  features: {
    title: ['Erlang-style actors,', 'compiled to native code.'],
    tiles: [
      { id: 'syntax', span: 7, dark: true, title: 'Reads like Elixir. Checks like ML.', line: 'Every value is statically typed, and inference means most code needs no annotations.' },
      { id: 'supervisor', span: 5, title: 'Supervisors restart what fails.', line: 'A panic ends only the actor it happens in.' },
      { id: 'pid', span: 4, title: 'A Pid knows its messages.', line: 'Send an actor the wrong type and the build fails.' },
      { id: 'binary', span: 4, title: 'One native binary.', line: 'LLVM compiles your program to an executable. There is no VM to install.' },
      { id: 'safety', span: 4, title: 'Safety you can see in the types.', line: 'A Plaintext<T> can\u2019t reach a log, and a resource moves at most once.' },
    ],
  },
  toolchain: {
    title: ['One toolchain.', 'meshc for code, meshpkg for packages.'],
    commands: [
      { id: 'build', command: 'meshc build .', gloss: 'Compiles through LLVM to a native executable.' },
      { id: 'test', command: 'meshc test', gloss: 'Test bodies run as actors and can assert_receive.' },
      { id: 'fmt', command: 'meshc fmt', gloss: 'The formatter ships in the compiler.' },
      { id: 'lint', command: 'meshc lint', gloss: 'The linter ships in the compiler too.' },
      { id: 'repl', command: 'meshc repl', gloss: 'A REPL backed by a JIT.' },
      { id: 'migrate', command: 'meshc migrate', gloss: 'Runs your database migrations.' },
      { id: 'plaintext-report', command: 'meshc plaintext-report .', gloss: 'Lists every Plaintext disclosure with its reason, so CI can review new ones.' },
      { id: 'meshpkg', command: 'meshpkg install', gloss: 'SHA-256 checked, exact-pinned in mesh.lock, and no build scripts ever run.' },
    ],
    note: 'Plus an LSP for VS Code and Neovim.',
  },
  quote: {
    text: '\u201cIf a language can\u2019t power its own apps, it\u2019s not ready for yours.\u201d',
    author: 'The Morse team',
    role: 'Makers of Morse, an end-to-end encrypted messenger',
  },
  close: { title: 'Install Mesh.' },
}

const MODULE_GROUPS = [
  { name: 'Collections and text', modules: ['String', 'List', 'Map', 'Set', 'Tuple', 'Range', 'Queue', 'Iter', 'Regex'] },
  { name: 'Values and encoding', modules: ['Bytes', 'U64', 'U128', 'I128', 'Checked', 'Json', 'Base64', 'Hex'] },
  { name: 'System and time', modules: ['IO', 'Env', 'File', 'Math', 'DateTime', 'Monotonic', 'Duration', 'Random', 'Crypto'] },
  { name: 'Concurrency', modules: ['Job', 'Timer', 'Channel', 'Process', 'Test'] },
  { name: 'Web', modules: ['HTTP', 'Request', 'Ws', 'Http', 'WsClient'] },
  { name: 'Databases', modules: ['Sqlite', 'Pg', 'Pool', 'Orm', 'Query', 'Repo', 'Changeset', 'Migration'] },
  { name: 'Distribution', modules: ['Node', 'Global', 'Continuity', 'Cluster'] },
]

const GROUP_OF = new Map(MODULE_GROUPS.flatMap((group) => group.modules.map((module) => [module, group.name] as const)))

// Literals stand in for the module that owns them, so `~r/…/` counts as Regex.
const LITERAL_MODULES: [RegExp, string][] = [[/~r\//, 'Regex'], [/\bjson \{/, 'Json'], [/deriving\(Json/, 'Json']]

function modulesIn(line: string): string[] {
  const found = [...line.matchAll(/\b([A-Z][A-Za-z0-9]*)\./g)].map((m) => m[1]).filter((name) => GROUP_OF.has(name))
  for (const [pattern, module] of LITERAL_MODULES) if (pattern.test(line)) found.push(module)
  return [...new Set(found)]
}

declare const data: LandingData
export { data }

export default defineLoader({
  async load(): Promise<LandingData> {
    const hl: HighlighterCore = await createHighlighter({
      themes: [meshLight as any, meshDark as any],
      langs: [{ ...meshGrammar, name: 'mesh' } as any],
    })
    const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    // Token spans carrying both palettes; CSS picks one per surface.
    const html = (code: string) =>
      hl.codeToTokens(code, { lang: 'mesh', themes: { light: 'mesh-light', dark: 'mesh-dark' }, defaultColor: false })
        .tokens.map((line) => line.map((t) => {
          const style = Object.entries(t.htmlStyle ?? {}).map(([k, v]) => `${k}:${v}`).join(';')
          return `<span style="${style}">${esc(t.content)}</span>`
        }).join(''))
        .join('\n')
    const dark = (line: string): DarkToken[] =>
      (hl.codeToTokensBase(line, { lang: 'mesh', theme: 'mesh-dark' })[0] ?? []).map((t) =>
        t.fontStyle && t.fontStyle & 1 ? [t.content, t.color!, 1] : [t.content, t.color!])

    const tokens: Record<string, DarkToken[]> = {}
    for (const step of STEPS) {
      for (const key of compileStep(step, FILES[step.file]).keys) tokens[key] ??= dark(key)
    }

    const result: LandingData = {
      ...COPY,
      demo: { steps: STEPS, files: FILES, tokens },
      art: {
        pipes: html(PIPES),
        supervisor: { root: 'WorkerSup', workers: ['w1', 'w2', 'w3'], hint: 'Tap a worker to crash it.' },
        pid: { code: 'E0014', message: 'message type mismatch', src: html(WRONG_SEND), label: '╰── expected Message, found String' },
        binary: {
          label: 'meshc build . prints Compiled: ./output. Running ./output prints total = 42.',
          rows: [
            { command: 'meshc build .' },
            { ok: 'Compiled:', output: ' ./output', dim: true },
            { command: './output' },
            { output: 'total = 42' },
          ],
        },
        safety: {
          label: 'A Plaintext value cannot flow to print, log, JSON, or an HTTP body. It leaves only through encryption or declassify.',
          source: 'Plaintext<String>',
          blocked: ['print', 'log', 'JSON', 'HTTP body'],
          allowed: ['encrypt', 'declassify'],
        },
      },
      distribution: {
        title: ['Clusters are part of the runtime.', 'Not another service to run.'],
        lead: 'Wrap a route in <code class="ic">HTTP.clustered</code> and the runtime picks the node that runs each request.',
        link: { text: 'Autonomous clusters', href: '/docs/autonomous-clusters/' },
        figure: {
          lg: clusterFigure(CALLOUTS, 'lg'),
          sm: clusterFigure([], 'sm'),
          label: 'A request enters the ingress node, the runtime runs it on an execution node, and its continuity record is replicated to a replica node',
        },
        code: html(ROUTER),
      },
      stdlib: {
        title: ['Batteries included.', 'Built in, with no import.'],
        file: 'events.mpl',
        lines: html(STDLIB).split('\n').map((lineHtml, i) => {
          const modules = modulesIn(STDLIB.split('\n')[i])
          return { html: lineHtml, modules, groups: [...new Set(modules.map((m) => GROUP_OF.get(m)!))] }
        }),
        groups: MODULE_GROUPS,
        link: { text: 'Standard library', href: '/docs/stdlib/' },
      },
    }
    hl.dispose()
    return result
  },
})
