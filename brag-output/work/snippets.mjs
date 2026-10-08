// Highlights the video's Mesh samples with the grammar and dark theme the site uses.
import { readFileSync, writeFileSync } from 'node:fs'
const root = '/Volumes/SSK-SSD/mesh-lang'
const { createHighlighter } = await import(`${root}/website/node_modules/shiki/dist/index.mjs`)
const grammar = JSON.parse(readFileSync(`${root}/tools/editors/vscode-mesh/syntaxes/mesh.tmLanguage.json`, 'utf8'))
const theme = JSON.parse(readFileSync(`${root}/website/docs/.vitepress/theme/shiki/mesh-dark.json`, 'utf8'))
const hl = await createHighlighter({ themes: [theme], langs: [{ ...grammar, name: 'mesh' }] })

// Lines as arrays of inner HTML so the page can reveal them one by one.
const lines = (code) => {
  const html = hl.codeToHtml(code, { lang: 'mesh', theme: 'mesh-dark' })
  return [...html.matchAll(/<span class="line">(.*?)<\/span>(?=\n|<\/code>)/gs)].map((m) => m[1])
}

// Landing samples (website/docs/.vitepress/theme/components/landing/landing.data.mts)
const counter = `actor counter(total :: Int) do
  receive do
    amount -> counter(total + amount)
  end
end

fn main() do
  let pid = spawn(counter, 0) # Pid<Int>
  send(pid, 5)
  send(pid, "five")`

const clustered = `let router = HTTP.router()
  # the runtime decides which node runs it
  |> HTTP.on_get("/hello/:name", HTTP.clustered(hello))`

const modules = {
  'HTTP client': `pub fn fetch_price(market :: String) -> String ! String do
  let request = Http.build(:get, "https://api.example.com/price")
    |> Http.query("market", market)
    |> Http.timeout(5_000)

  case Http.send(request) do
    Ok(response) -> Ok(response.body)
    Err(error) -> Err(error)
  end
end`,
  WebSockets: `fn on_connect(conn, _path, _headers) -> Int do
  Ws.join(conn, "updates")
  1
end

fn on_message(_conn, msg :: String) do
  Ws.broadcast("updates", msg)
end`,
  Postgres: `fn list_open(pool :: PoolHandle) do
  Pool.query(pool,
    "select * from todos where completed = $1",
    ["false"])
end`,
  SQLite: `fn record_event(db :: SqliteConn, kind :: String) -> Int ! String do
  Sqlite.execute(db,
    "insert into events (kind) values (?1)",
    [kind])
end`,
  JSON: `fn title_from_body(body :: String) -> String ! String do
  let root = Json.parse(body)?
  let title = Json.object_get(root, "title")?
  Json.as_string(title)
end`,
  Jobs: `fn load_report() -> String ! String do
  let job = Job.async(fn -> build_report() end)
  Job.await_timeout(job, 1000)
end`,
  Testing: `fn slugify(title :: String) -> String do
  title |> String.to_lower() |> String.replace(" ", "-")
end

describe("slugify") do
  test("lowercases and joins words") do
    assert_eq(slugify("Ship a Fleet"), "ship-a-fleet")
  end
end`,
}

const out = {
  counter: lines(counter),
  clustered: lines(clustered),
  modules: Object.entries(modules).map(([name, code]) => ({ name, lines: lines(code) })),
}
writeFileSync(new URL('./snippets.js', import.meta.url), `window.SNIPPETS = ${JSON.stringify(out)};\n`)
console.log(out.counter.length, out.clustered.length, out.modules.map((m) => m.lines.length).join(','))
console.log(out.counter[9])
