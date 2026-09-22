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
  const socialImageSource = read('website/scripts/generate-og-image.py')
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

  assert.match(readme, /version-v14\.0/)
  assert.match(readme, /\.\/output/)
  assert.doesNotMatch(readme, /AUTONOMOUS-SCALING-AND-LOAD-BALANCING-PLAN/)
  assert.match(gettingStarted, /\.\/output/)
  assert.match(clusteredExample, /\.\/output/)
  assert.doesNotMatch(`${gettingStarted}\n${clusteredExample}`, /\.\/(?:hello_mesh|hello_cluster|hello)\b/)
  assert.match(typeSystem, /type Pair<A, B> = \(A, B\)/)
  assert.doesNotMatch(`${typeSystem}\n${languageBasics}`, /Type aliases (?:are not generic|in v13\.0 are non-generic)/)
  assert.doesNotMatch(iterators, /Iter\.from\(\).*works with lists, maps, and sets/)
  assert.doesNotMatch(`${typeSystem}\n${languageBasics}\n${iterators}`, /\bSet<T>/)
  assert.match(languageBasics, /Unicode alphabetic code point/)
  assert.match(languageBasics, /Reserved keywords are exact ASCII words/)
  assert.match(reference, /Unicode alphanumeric code point/)
  assert.match(languageBasics, /The pattern itself may span physical source lines/)
  assert.match(reference, /may span physical source lines until an unescaped `\/`/)
  assert.match(typeSystem, /Struct \| `Debug`, `Eq`, `Ord`, `Hash`/)
  assert.match(typeSystem, /Sum type \| `Debug`, `Eq`, `Ord`/)
  assert.doesNotMatch(`${languageBasics}\n${reference}`, /value\[index\]/)
  assert.match(languageBasics, /direct call to the current function in tail position is lowered to a loop/i)
  assert.match(reference, /native code generation executes\s+only the first arm/i)
  assert.match(concurrency, /\(new_state, reply\)/)
  assert.doesNotMatch(concurrency, /\(reply, new_state\)/)
  assert.doesNotMatch(web, /Ws\.serve_tls/)
  assert.doesNotMatch(`${testing}\n${docs}`, /String\.downcase|IO\.puts|\bpanic\(/)
  assert.match(testing, /assert_raises[\s\S]+assert\(false\)/)
  assert.doesNotMatch(landing, /Pg\.query\(pool|Env\.get\("DATABASE_URL"\)|Ws\.broadcast\(conn/)
  assert.doesNotMatch(landing, /Sqlite\.open\("app\.db"\)/)
  assert.match(landing, /db :: SqliteConn/)
  assert.doesNotMatch(landing, /from [^\n]+ import [^(\n]*,\n/)
  assert.match(landing, /Ws\.join\(conn, "updates"\)[\s\S]*?\n  1/)
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

test('every sidebar icon named in the config is imported', () => {
  const config = read('website/docs/.vitepress/config.mts')
  const sidebarItem = read('website/docs/.vitepress/theme/components/docs/DocsSidebarItem.vue')
  const icons = [...config.matchAll(/icon: '(\w+)'/g)].map(([, icon]) => icon)

  assert.ok(icons.length > 0)
  for (const icon of icons) {
    assert.match(sidebarItem, new RegExp(`\\b${icon}\\b`), `DocsSidebarItem.vue does not import sidebar icon ${icon}`)
  }
})
