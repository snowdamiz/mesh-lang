import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const read = (relativePath) => fs.readFileSync(path.join(root, relativePath), 'utf8')

test('public docs cover the current Mesh surface', () => {
  const readme = read('README.md')
  const config = read('website/docs/.vitepress/config.mts')
  const gettingStarted = read('website/docs/docs/getting-started/index.md')
  const clusteredExample = read('website/docs/docs/getting-started/clustered-example/index.md')
  const typeSystem = read('website/docs/docs/type-system/index.md')
  const languageBasics = read('website/docs/docs/language-basics/index.md')
  const iterators = read('website/docs/docs/iterators/index.md')
  const concurrency = read('website/docs/docs/concurrency/index.md')
  const reference = read('website/docs/docs/reference/index.md')
  const web = read('website/docs/docs/web/index.md')
  const stdlib = read('website/docs/docs/stdlib/index.md')
  const testing = read('website/docs/docs/testing/index.md')
  const home = read('website/docs/index.md')
  const landing = read('website/docs/.vitepress/theme/components/landing/landing.data.mts')
  const socialImageSource = ['website/scripts/og-image.html', 'website/scripts/generate-og-image.mjs'].map(read).join('\n')
  const docs = [
    'website/docs/docs/language-basics/index.md',
    'website/docs/docs/type-system/index.md',
    'website/docs/docs/concurrency/index.md',
    'website/docs/docs/stdlib/index.md',
    'website/docs/docs/web/index.md',
    'website/docs/docs/databases/index.md',
    'website/docs/docs/native-packages/index.md',
    'website/docs/docs/packages/index.md',
    'website/docs/docs/tooling/index.md',
    'website/docs/docs/reference/index.md',
  ].map(read).join('\n')

  assert.match(readme, /img\.shields\.io\/github\/v\/release\/hyperpush-org\/mesh-lang/)
  assert.match(readme, /\.\/output/)
  assert.doesNotMatch(readme, /AUTONOMOUS-SCALING-AND-LOAD-BALANCING-PLAN/)
  assert.match(gettingStarted, /\.\/output/)
  assert.match(clusteredExample, /\.\/output/)
  assert.doesNotMatch(`${gettingStarted}\n${clusteredExample}`, /\.\/(?:hello_mesh|hello_cluster|hello)\b/)
  assert.match(typeSystem, /type Pair<A, B> = \(A, B\)/)
  assert.doesNotMatch(`${typeSystem}\n${languageBasics}`, /Type aliases (?:are not generic|in v13\.0 are non-generic)/)
  // Sets and queues are generic, and Iter.from takes any built-in collection.
  assert.match(typeSystem, /\bSet<T>/)
  assert.match(typeSystem, /\bQueue<T>/)
  assert.match(concurrency, /\bChannel<T>/)
  assert.doesNotMatch(reference, /Channel` payloads are currently `Int`/)
  assert.match(iterators, /`Iter\.from\(source\)`.*`Map<K, V>`/)
  assert.match(languageBasics, /Unicode alphabetic code point/)
  assert.match(languageBasics, /Reserved keywords are exact ASCII words/)
  assert.match(reference, /Unicode alphanumeric code point/)
  assert.match(languageBasics, /The pattern itself may span physical source lines/)
  assert.match(reference, /may span physical source lines until an unescaped `\/`/)
  assert.match(typeSystem, /Struct \| `Debug`, `Eq`, `Ord`, `Hash`/)
  assert.match(typeSystem, /Sum type \| `Debug`, `Eq`, `Ord`/)
  assert.doesNotMatch(`${languageBasics}\n${reference}`, /value\[index\]/)
  assert.match(languageBasics, /direct call to the current function in tail position is lowered to a loop/i)
  assert.match(reference, /Receive arms match the next message like\s+`case` arms/)
  assert.doesNotMatch(reference, /dispatches only its first arm/)
  assert.match(concurrency, /\(new_state, reply\)/)
  assert.doesNotMatch(concurrency, /\(reply, new_state\)/)
  assert.match(web, /Ws\.serve_tls\(on_connect, on_message, on_close, port, "cert\.pem", "key\.pem"\)/)
  assert.doesNotMatch(`${testing}\n${docs}`, /String\.downcase|IO\.puts/)
  assert.match(testing, /assert_raises[\s\S]+assert\(false\)/)
  assert.match(landing, /let pid = spawn\(counter, 0\)/)
  assert.match(landing, /actor client\(pid :: Pid<Message>\) do/)
  assert.match(landing, /HTTP\.clustered\(2, show_account\)/)
  assert.match(landing, /@cluster\(3\)\npub fn rebuild_index\(\)/)
  assert.doesNotMatch(landing, /Pg\.query\(pool|Env\.get\("DATABASE_URL"\)|Ws\.broadcast\(conn/)
  assert.doesNotMatch(landing, /Sqlite\.open\("app\.db"\)/)
  assert.doesNotMatch(landing, /from [^\n]+ import [^(\n]*,\n/)
  assert.doesNotMatch(landing, /Cluster\.telemetry\(\)[^\n]+continuity/)
  assert.doesNotMatch(`${home}\n${config}\n${socialImageSource}`, /Repo\.find|Continuity\.submit\(key, process_order\)|One public app URL/)
  assert.match(stdlib, /from_unix_(?:ms|secs).+Result<DateTime,\s*String>/s)

  for (const link of ['/docs/native-packages/', '/docs/packages/', '/docs/reference/']) {
    assert.match(config, new RegExp(link.replaceAll('/', '\\/')))
  }

  for (const surface of [
    '@cluster',
    '@native',
    'tail position',
    'Bytes.from_base58',
    'U128.multiply',
    'Checked.mul_div',
    'Monotonic.elapsed',
    'Channel.bounded_bytes',
    'Random.next_int',
    'Process.install_shutdown_signals',
    'Job.await_timeout',
    'Json.object_get',
    'Http.max_redirects',
    'Http.stage_timeout',
    'WsClient.connect',
    'mesh-borsh',
    'mesh-anchor',
    'mesh-solana',
    'compile_message_v0',
    'simulate_transaction_request',
  ]) {
    assert.ok(docs.includes(surface), `missing current public surface: ${surface}`)
  }
})

const markdownUnder = (dir) => fs.readdirSync(path.join(root, dir), { withFileTypes: true }).flatMap((entry) => {
  const relativePath = path.join(dir, entry.name)
  if (entry.isDirectory()) return ['.vitepress', 'public', 'node_modules'].includes(entry.name) ? [] : markdownUnder(relativePath)
  return entry.name.endsWith('.md') ? [relativePath] : []
})

// meshc rejects declared work with parameters: the runtime owns its continuity metadata.
test('every @cluster sample decorates a public function with no parameters', () => {
  const DECLARATION = /@cluster(?:\(\d+\))?\s+(pub\s+)?(?:fn|def)\s+\w+\(([^)]*)\)/g
  const LANDING = 'website/docs/.vitepress/theme/components/landing/landing.data.mts'
  // The landing data keeps each Mesh sample in a top-level template-literal constant.
  const landingSamples = new Map([...read(LANDING).matchAll(/^const ([A-Z_]+) = `([\s\S]*?)`$/gm)].map(([, name, code]) => [name, code]))
  for (const name of ['COUNTER', 'CLUSTER', 'PIPES', 'ROUTER', 'STDLIB', 'WRONG_SEND']) {
    assert.ok(landingSamples.has(name), `${LANDING}: no Mesh sample constant ${name}, so this test reads none of its code`)
  }
  const meshCode = (source, text) => (source === LANDING ? [...landingSamples.values()] : [...text.matchAll(/^```mesh\n([\s\S]*?)^```/gm)].map(([, code]) => code))
  const surfaces = {
    README: ['README.md'],
    'landing page': [LANDING],
    docs: markdownUnder('website/docs'),
  }
  for (const [surface, sources] of Object.entries(surfaces)) {
    let declarations = 0
    for (const source of sources) {
      const text = read(source)
      // Inline samples in prose, like `@cluster pub fn sync_todos()`, are held to the same rules as code.
      for (const [declaration, pub, params] of text.matchAll(DECLARATION)) {
        assert.ok(pub, `${source}: @cluster decorates a public function: ${declaration}`)
        assert.equal(params.trim(), '', `${source}: @cluster functions take no parameters: ${declaration}`)
      }
      // Prose names `@cluster` freely; in Mesh code every one must decorate a function.
      for (const code of meshCode(source, text)) {
        const found = [...code.matchAll(DECLARATION)].length
        assert.equal(found, (code.match(/@cluster\b/g) ?? []).length, `${source}: every @cluster in Mesh code decorates a function`)
        declarations += found
      }
    }
    assert.ok(declarations > 0, `the ${surface} shows no @cluster sample, so this test reads none of its Mesh code`)
  }
})

test('every docs link in the sidebar and site chrome resolves to a page', () => {
  const sources = [
    'website/docs/.vitepress/config.mts',
    'website/docs/.vitepress/theme/site.ts',
    'website/docs/.vitepress/theme/components/NotFoundPage.vue',
    'website/docs/.vitepress/theme/components/landing/landing.data.mts',
  ].map(read).join('\n')
  const links = new Set([...sources.matchAll(/['"`](\/docs\/[\w-][\w\-/]*)['"`#]/g)].map(([, link]) => link))

  assert.ok(links.size > 20, `expected the sidebar's links, found ${links.size}`)
  for (const link of links) {
    const page = path.join('website/docs', link, 'index.md')
    assert.ok(fs.existsSync(path.join(root, page)), `${link} has no page at ${page}`)
  }
})
