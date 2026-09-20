<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useScrollReveal } from '@/composables/useScrollReveal'
import { Radar } from 'lucide-vue-next'
import { getHighlighter, highlightCode } from '@/composables/useShiki'

const { observe } = useScrollReveal()
const root = ref<HTMLElement>()
const specimenEl = ref<HTMLElement>()

interface Module {
  name: string
  desc: string
  file: string
  code: string
}

const modules: Module[] = [
  {
    name: 'HTTP',
    desc: 'Server, router, middleware',
    file: 'api/router.mpl',
    code: `pub fn build_router() do
  HTTP.router()
    |> HTTP.on_get("/todos", handle_list_todos)
    |> HTTP.on_get("/todos/:id", handle_get_todo)
    |> HTTP.on_post("/todos", handle_create_todo)
end`,
  },
  {
    name: 'WebSockets',
    desc: 'Connections, rooms, broadcast',
    file: 'api/live.mpl',
    code: `fn on_connect(conn, _path, _headers) -> Int do
  let _ = Ws.join(conn, "updates")
  1
end

fn on_message(_conn, msg :: String) do
  let _ = Ws.broadcast("updates", msg)
  nil
end

fn on_close(_conn, _code, _reason) do
  println("socket closed")
end`,
  },
  {
    name: 'Postgres',
    desc: 'Driver and connection pool',
    file: 'storage/todos.mpl',
    code: `fn list_open(pool :: PoolHandle) do
  Pool.query(pool,
    "select * from todos where completed = $1",
    ["false"])
end`,
  },
  {
    name: 'SQLite',
    desc: 'Caller-owned embedded connection',
    file: 'storage/local.mpl',
    code: `fn record_event(db :: SqliteConn, kind :: String) -> Int ! String do
  Sqlite.execute(db,
    "insert into events (kind) values (?1)",
    [kind])
end`,
  },
  {
    name: 'Query',
    desc: 'Composable query builder',
    file: 'storage/queries.mpl',
    code: `fn open_todos() do
  Query.from("todos")
    |> Query.where(:completed, "false")
    |> Query.order_by(:created_at, :desc)
    |> Query.limit(20)
end`,
  },
  {
    name: 'Migrations',
    desc: 'Schema versioning',
    file: 'migrations/create_todos.mpl',
    code: `pub fn up(pool :: PoolHandle) -> Int ! String do
  Migration.create_table(pool, "todos",
    ["id:UUID:PRIMARY KEY",
     "title:TEXT:NOT NULL",
     "completed:BOOLEAN:DEFAULT false"]) ?
  Ok(0)
end`,
  },
  {
    name: 'Bytes',
    desc: 'Binary-safe values and codecs',
    file: 'protocol/frame.mpl',
    code: `fn decode_key(encoded :: String) -> Bytes ! String do
  let raw = Bytes.from_base58(encoded)?
  if Bytes.length(raw) == 32 do
    Ok(raw)
  else
    Err("expected a 32-byte key")
  end
end`,
  },
  {
    name: 'Jobs',
    desc: 'Typed asynchronous results',
    file: 'workers/report.mpl',
    code: `fn load_report() -> String ! String do
  let job = Job.async(fn -> build_report() end)
  case Job.await_timeout(job, 1000) do
    Ok(report) -> Ok(report)
    Err(reason) -> Err(reason)
  end
end`,
  },
  {
    name: 'Actors',
    desc: 'Typed processes, supervision',
    file: 'services/counter.mpl',
    code: `actor counter() do
  receive do
    msg -> println("count: #{msg}")
  end
end

let pid = spawn(counter)
send(pid, 1)`,
  },
  {
    name: 'JSON',
    desc: 'Encode / decode',
    file: 'api/todos.mpl',
    code: `fn todo_to_json(todo :: Todo) -> String do
  Json.encode(todo)
end

fn title_from_body(body :: String) -> String ! String do
  let root = Json.parse(body)?
  let title = Json.object_get(root, "title")?
  Json.as_string(title)
end`,
  },
  {
    name: 'Env',
    desc: 'Config from the environment',
    file: 'config.mpl',
    code: `fn database_url() -> String do
  Env.get("DATABASE_URL", "postgres://localhost/app")
end

fn missing_required_env(name :: String) -> String do
  "Missing required environment variable #{name}"
end`,
  },
  {
    name: 'Testing',
    desc: 'Runner in the toolchain',
    file: 'tests/todos_test.mpl',
    code: `describe("todos") do
  test("completes a todo") do
    let todo = create_todo("write docs")
    let toggled = toggle_todo(todo.id)
    assert_eq(toggled.completed, true)
  end
end`,
  },
]

const active = ref(0)
const highlighted = ref<string[]>([])

let timer: ReturnType<typeof setInterval> | null = null
let io: IntersectionObserver | null = null
let visible = false
let interacted = false

function select(i: number) {
  active.value = i
  interacted = true
}

onMounted(async () => {
  root.value?.querySelectorAll('.reveal, .reveal-zoom, .reveal-stagger').forEach((el) => observe(el))

  // Auto-walk the index until the visitor takes over
  if (specimenEl.value) {
    io = new IntersectionObserver(
      (entries) => {
        visible = entries[0].isIntersecting
      },
      { threshold: 0.3 },
    )
    io.observe(specimenEl.value)
  }
  timer = setInterval(() => {
    if (visible && !interacted) active.value = (active.value + 1) % modules.length
  }, 2600)

  try {
    const hl = await getHighlighter()
    highlighted.value = modules.map((m) => highlightCode(hl, m.code))
  } catch {
    // Highlighting failed -- raw code fallback remains visible
  }
})

onUnmounted(() => {
  if (timer) clearInterval(timer)
  io?.disconnect()
  io = null
})
</script>

<template>
  <section class="mx-auto max-w-6xl px-4 py-20 sm:px-6 md:py-28">
    <div ref="root">
      <span class="l-eyebrow reveal">standard library</span>

      <div class="reveal reveal-d1 mt-6 flex flex-wrap items-end justify-between gap-6">
        <h2 class="font-display text-4xl font-extrabold leading-[1.05] text-foreground sm:text-[2.75rem]">
          Server primitives<br /><em class="l-fancy">included.</em>
        </h2>
        <p class="max-w-sm text-base leading-relaxed text-muted-foreground">
          HTTP, WebSockets, database drivers, structured JSON, actors, jobs, binary values, and testing ship with the
          toolchain. Use packages for ecosystem-specific protocols and native libraries.
        </p>
      </div>

      <!-- Library index + live specimen -->
      <div class="mt-12 grid items-start gap-10 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)] lg:gap-14">
        <!-- The index: a table of contents, not a grid of cards -->
        <ul class="reveal-stagger divide-y divide-border/70 border-y border-border/70">
          <li v-for="(mod, i) in modules" :key="mod.name">
            <button
              type="button"
              class="group relative flex w-full items-baseline gap-3 px-3 py-[9px] text-left transition-colors sm:gap-4"
              :class="active === i ? 'bg-foreground/[0.035]' : 'hover:bg-foreground/[0.025]'"
              @mouseenter="select(i)"
              @focus="select(i)"
              @click="select(i)"
            >
              <!-- active marker -->
              <span
                class="absolute inset-y-0 left-0 w-0.5 transition-opacity"
                :style="{ background: 'var(--l-accent)', opacity: active === i ? 1 : 0 }"
              />
              <span
                class="w-6 shrink-0 font-mono text-[11px] tabular-nums"
                :class="active === i ? 'text-[var(--l-accent)]' : 'text-muted-foreground/50'"
              >{{ String(i + 1).padStart(2, '0') }}</span>
              <span
                class="shrink-0 font-mono text-[13.5px] font-bold transition-colors"
                :class="active === i ? 'text-[var(--l-accent)]' : 'text-foreground'"
              >{{ mod.name }}</span>
              <!-- dot leaders -->
              <span class="mx-1 flex-1 -translate-y-[3px] border-b border-dotted border-border" aria-hidden="true" />
              <span class="shrink-0 text-right text-xs text-muted-foreground sm:text-[13px]">{{ mod.desc }}</span>
            </button>
          </li>
        </ul>

        <!-- The specimen: the selected module, doing its job -->
        <div ref="specimenEl" class="reveal-zoom reveal-d2 l-window lg:sticky lg:top-24">
          <div class="l-window-head">
            <span class="l-window-dots" aria-hidden="true"><span /><span /><span /></span>
            <span class="min-w-0 truncate">{{ modules[active].file }}</span>
            <span class="shrink-0 font-mono text-[10px] tracking-[0.1em] text-[var(--l-accent)]">{{ modules[active].name.toLowerCase() }}</span>
          </div>
          <div class="min-h-[240px]">
            <div
              v-if="highlighted[active]"
              v-html="highlighted[active]"
              class="vp-code landing-code w-full max-w-full font-mono"
            />
            <pre
              v-else
              class="max-w-full overflow-x-auto px-6 py-4 font-mono text-[0.8125rem] leading-[1.9] text-foreground"><code>{{ modules[active].code }}</code></pre>
          </div>
          <div class="flex items-center justify-between border-t border-border px-5 py-2.5 font-mono text-[11px] text-muted-foreground">
            <span>module <span class="text-foreground">{{ String(active + 1).padStart(2, '0') }}</span> / {{ modules.length }}</span>
            <span>built in</span>
          </div>
        </div>
      </div>

      <!-- Observatory strip -->
      <div
        class="reveal mt-12 flex flex-col gap-4 rounded-xl border px-6 py-5 sm:flex-row sm:items-center sm:gap-5"
        style="border-color: color-mix(in oklab, var(--warn) 35%, var(--border)); background: color-mix(in oklab, var(--warn) 6%, transparent);"
      >
        <span
          class="flex size-10 shrink-0 items-center justify-center rounded-lg"
          :style="{ background: 'color-mix(in oklab, var(--warn) 14%, transparent)', color: 'var(--warn)' }"
        >
          <Radar class="size-5" />
        </span>
        <p class="text-sm leading-relaxed text-muted-foreground">
          <span class="font-semibold text-foreground">Bounded runtime telemetry</span>
          — <span class="font-mono">Cluster.telemetry()</span> exposes scheduler, mailbox, HTTP, dispatch, process, and
          resource counters; <span class="font-mono">meshc cluster</span> provides the operator and continuity views.
        </p>
        <span
          class="inline-flex w-fit shrink-0 items-center gap-2 rounded-md px-2.5 py-1 font-mono text-[10.5px] font-bold tracking-[0.1em] sm:ml-auto"
          :style="{ background: 'color-mix(in oklab, var(--warn) 16%, transparent)', color: 'var(--warn)' }"
        >
          <span class="l-blink size-1.5 rounded-full" :style="{ backgroundColor: 'var(--warn)' }" />
          available
        </span>
      </div>
    </div>
  </section>
</template>
