// Every Mesh sample on the landing page, highlighted once at build time with
// the grammar and themes the docs use, so the page ships no highlighter.
import { defineLoader } from 'vitepress'
import { createHighlighter } from 'shiki'
import meshGrammar from '../../../../../../tools/editors/vscode-mesh/syntaxes/mesh.tmLanguage.json'
import meshLight from '../../shiki/mesh-light.json'
import meshDark from '../../shiki/mesh-dark.json'

const hero = {
  file: 'main.mpl',
  // 1-based lines to call out
  highlight: [10, 11],
  code: `pub fn hello(request :: Request) -> Response do
  case Request.param(request, "name") do
    Some(name) -> HTTP.response(200, json { hello: name })
    None -> HTTP.response(400, "missing name")
  end
end

fn main() do
  let router = HTTP.router()
    # the runtime decides which node runs it
    |> HTTP.on_get("/hello/:name", HTTP.clustered(hello))

  case Node.start_from_env() do
    Ok(_) -> HTTP.serve(router, 8080)
    Err(reason) -> println("boot failed: #{reason}")
  end
end`,
}

const modules = [
  {
    name: 'Actors',
    file: 'services/counter.mpl',
    href: '/docs/concurrency/',
    highlight: [10],
    code: `actor counter(total :: Int) do
  receive do
    amount -> counter(total + amount)
  end
end

fn main() do
  let pid = spawn(counter, 0) # Pid<Int>
  send(pid, 5)
  send(pid, "five") # compile error: expected Int, found String
end`,
  },
  {
    name: 'HTTP client',
    file: 'clients/prices.mpl',
    href: '/docs/web/#http-client',
    code: `pub fn fetch_price(market :: String) -> String ! String do
  let request = Http.build(:get, "https://api.example.com/price")
    |> Http.query("market", market)
    |> Http.timeout(5_000)

  case Http.send(request) do
    Ok(response) -> Ok(response.body)
    Err(error) -> Err(error)
  end
end`,
  },
  {
    name: 'WebSockets',
    file: 'api/live.mpl',
    href: '/docs/web/#websocket',
    code: `fn on_connect(conn, _path, _headers) -> Int do
  let _ = Ws.join(conn, "updates")
  1
end

fn on_message(_conn, msg :: String) do
  let _ = Ws.broadcast("updates", msg)
  nil
end`,
  },
  {
    name: 'Postgres',
    file: 'storage/todos.mpl',
    href: '/docs/databases/#postgresql-connections-and-pools',
    code: `fn list_open(pool :: PoolHandle) do
  Pool.query(pool,
    "select * from todos where completed = $1",
    ["false"])
end`,
  },
  {
    name: 'SQLite',
    file: 'storage/events.mpl',
    href: '/docs/databases/#sqlite',
    code: `fn record_event(db :: SqliteConn, kind :: String) -> Int ! String do
  Sqlite.execute(db,
    "insert into events (kind) values (?1)",
    [kind])
end`,
  },
  {
    name: 'JSON',
    file: 'api/todos.mpl',
    href: '/docs/web/#json',
    code: `fn title_from_body(body :: String) -> String ! String do
  let root = Json.parse(body)?
  let title = Json.object_get(root, "title")?
  Json.as_string(title)
end`,
  },
  {
    name: 'Jobs',
    file: 'workers/report.mpl',
    href: '/docs/concurrency/#jobs',
    code: `fn load_report() -> String ! String do
  let job = Job.async(fn -> build_report() end)
  case Job.await_timeout(job, 1000) do
    Ok(report) -> Ok(report)
    Err(reason) -> Err(reason)
  end
end`,
  },
  {
    name: 'Testing',
    file: 'tests/slug.test.mpl',
    href: '/docs/testing/',
    code: `fn slugify(title :: String) -> String do
  title |> String.to_lower() |> String.replace(" ", "-")
end

describe("slugify") do
  test("lowercases and joins words") do
    assert_eq(slugify("Ship a Fleet"), "ship-a-fleet")
  end
end`,
  },
]

export interface Sample {
  file: string
  html: string
}

export interface Module extends Sample {
  name: string
  href: string
}

export interface Data {
  hero: Sample
  modules: Module[]
}

declare const data: Data
export { data }

export default defineLoader({
  async load(): Promise<Data> {
    const hl = await createHighlighter({
      themes: [meshLight as any, meshDark as any],
      langs: [{ ...meshGrammar, name: 'mesh' } as any],
    })
    const render = ({ code, highlight = [] }: { code: string; highlight?: number[] }) =>
      hl.codeToHtml(code, {
        lang: 'mesh',
        themes: { light: 'mesh-light', dark: 'mesh-dark' },
        defaultColor: false,
        transformers: [
          {
            line(node, line) {
              if (highlight.includes(line)) this.addClassToHast(node, 'is-hl')
            },
          },
        ],
      })

    return {
      hero: { file: hero.file, html: render(hero) },
      modules: modules.map(({ code, highlight, ...meta }) => ({ ...meta, html: render({ code, highlight }) })),
    }
  },
})
