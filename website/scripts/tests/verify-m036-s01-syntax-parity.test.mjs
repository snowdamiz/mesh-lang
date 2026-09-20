import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(scriptDir, '..', '..', '..')
const corpusPath = path.join(root, 'scripts/fixtures/m036-s01-syntax-corpus.json')
const sharedGrammarPath = path.join(root, 'tools/editors/vscode-mesh/syntaxes/mesh.tmLanguage.json')
const shikiLightThemePath = path.join(root, 'website/docs/.vitepress/theme/shiki/mesh-light.json')
const shikiDarkThemePath = path.join(root, 'website/docs/.vitepress/theme/shiki/mesh-dark.json')
const clusterDecoratorFixturePath = path.join(root, 'scripts/fixtures/cluster-decorators.mpl')
const currentSyntaxFixturePath = path.join(root, 'scripts/fixtures/current-syntax-surface.mpl')
const compilerTokenPath = path.join(root, 'compiler/mesh-common/src/token.rs')

const BEGIN_SCOPE = 'punctuation.section.interpolation.begin.mesh'
const END_SCOPE = 'punctuation.section.interpolation.end.mesh'
const META_SCOPE = 'meta.interpolation.mesh'
const ANNOTATION_PUNCTUATION_SCOPE = 'punctuation.definition.annotation.mesh'
const CLUSTER_DECORATOR_SCOPE = 'storage.modifier.annotation.cluster.mesh'
const INTEGER_SCOPE = 'constant.numeric.integer.mesh'
const VARIABLE_SCOPE = 'variable.other.mesh'
const USER_TYPE_SCOPE = 'entity.name.type.mesh'
const LANGUAGE_CONSTANT_SCOPE = 'constant.language.mesh'
const KEYWORD_SCOPE_PREFIX = 'keyword.'
const OPERATOR_SCOPE_PREFIX = 'keyword.operator.'
const PUNCTUATION_SCOPE_PREFIX = 'punctuation.'
const TYPE_SCOPE = 'support.type.mesh'
const FUNCTION_SCOPE = 'entity.name.function.mesh'
const MODULE_SCOPE = 'entity.name.type.module.mesh'
const NATIVE_DECORATOR_SCOPE = 'storage.modifier.annotation.native.mesh'
const ORM_SCOPE = 'keyword.other.orm.mesh'
const SUPERVISOR_SCOPE = 'keyword.other.supervisor.mesh'
const SUPERVISOR_VALUE_SCOPE = 'constant.language.supervisor.mesh'
const WILDCARD_SCOPE = 'variable.language.wildcard.mesh'
const BLOCK_COMMENT_SCOPE = 'comment.block.mesh'
const LINE_COMMENT_SCOPE = 'comment.line.hash.mesh'
const DOC_COMMENT_SCOPE = 'comment.line.documentation.mesh'
const MODULE_DOC_COMMENT_SCOPE = 'comment.line.documentation.module.mesh'
const REGEX_SCOPE = 'string.regexp.mesh'
const ATOM_SCOPE = 'constant.language.atom.mesh'
const BUILTIN_CONSTRUCTOR_SCOPE = 'support.function.mesh'
const INTEGER_SCOPE_BY_BASE = {
  decimal: 'constant.numeric.integer.mesh',
  hex: 'constant.numeric.hex.mesh',
  binary: 'constant.numeric.binary.mesh',
  octal: 'constant.numeric.octal.mesh',
}
const FLOAT_SCOPE = 'constant.numeric.float.mesh'
const STRING_SCOPE_BY_KIND = {
  double: 'string.quoted.double.mesh',
  triple: 'string.quoted.triple.mesh',
}

function readText(absolutePath, label) {
  if (!fs.existsSync(absolutePath)) {
    throw new Error(`[m036-s01] missing ${label}: ${path.relative(root, absolutePath)}`)
  }
  return fs.readFileSync(absolutePath, 'utf8')
}

function readJson(absolutePath, label) {
  return JSON.parse(readText(absolutePath, label))
}

async function importRepoModule(relativePath, label) {
  const absolutePath = path.join(root, relativePath)
  if (!fs.existsSync(absolutePath)) {
    throw new Error(`[m036-s01] missing ${label}: ${relativePath}`)
  }
  return import(pathToFileURL(absolutePath).href)
}

async function withTimeout(label, timeoutMs, promiseFactory) {
  let timeoutId
  const timeoutPromise = new Promise((_, reject) => {
    timeoutId = setTimeout(() => reject(new Error(`[m036-s01] ${label} timed out after ${timeoutMs}ms`)), timeoutMs)
  })

  try {
    return await Promise.race([Promise.resolve().then(promiseFactory), timeoutPromise])
  } finally {
    clearTimeout(timeoutId)
  }
}

function relativePath(absolutePath) {
  return path.relative(root, absolutePath).replace(/\\/g, '/')
}

function offsetToLineColumn(text, offset) {
  const normalized = text.slice(0, offset).replace(/\r\n/g, '\n')
  const lines = normalized.split('\n')
  return {
    line: lines.length,
    column: lines.at(-1).length + 1,
  }
}

function formatRange(text, start, end) {
  const startPos = offsetToLineColumn(text, start)
  const endPos = offsetToLineColumn(text, end)
  return `${startPos.line}:${startPos.column}-${endPos.line}:${endPos.column}`
}

function findRequiredOffset(text, search, label, filePath) {
  const offset = text.indexOf(search)
  if (offset === -1) {
    throw new Error(`[m036-s01] cluster decorator fixture drift: missing ${label} ${JSON.stringify(search)} in ${filePath}`)
  }
  return offset
}

function nthOffset(text, search, occurrence, label, filePath) {
  let offset = -1
  for (let index = 0; index <= occurrence; index += 1) {
    offset = text.indexOf(search, offset + 1)
    if (offset === -1) {
      throw new Error(`[m036-s01] syntax fixture drift: missing ${label} occurrence=${occurrence + 1} ${JSON.stringify(search)} in ${filePath}`)
    }
  }
  return offset
}

function scopeCase(snippet, filePath, definition) {
  const searchStart = nthOffset(
    snippet,
    definition.search,
    definition.occurrence ?? 0,
    definition.id,
    filePath,
  )
  const token = definition.token ?? definition.search
  const tokenOffset = definition.tokenOffset ?? definition.search.indexOf(token)
  assert.ok(tokenOffset >= 0, `case ${definition.id} token must occur inside its search text`)
  const start = searchStart + tokenOffset
  return {
    ...definition,
    start,
    end: start + token.length,
  }
}

function lineSlice(text, startLine, endLine) {
  const lines = text.split(/\r?\n/)
  const startIndex = startLine - 1
  const endIndex = endLine
  return lines.slice(startIndex, endIndex).join('\n')
}

function scanInterpolations(code, caseDef) {
  const matches = []
  for (let index = 0; index < code.length - 1; index += 1) {
    const opener = code.slice(index, index + 2)
    if (opener !== '#{' && opener !== '${') continue

    const form = opener === '#{' ? 'hash' : 'dollar'
    let braceDepth = 0
    let cursor = index + 2
    for (; cursor < code.length; cursor += 1) {
      const char = code[cursor]
      if (char === '{') {
        braceDepth += 1
      } else if (char === '}') {
        if (braceDepth === 0) break
        braceDepth -= 1
      }
    }

    if (cursor >= code.length) {
      throw new Error(`[m036-s01] corpus case ${caseDef.id} (${caseDef.path}) has an unterminated ${form} interpolation`)
    }

    matches.push({
      form,
      opener,
      start: index,
      openEnd: index + 2,
      exprStart: index + 2,
      exprEnd: cursor,
      endStart: cursor,
      endEnd: cursor + 1,
      expression: code.slice(index + 2, cursor),
      text: code.slice(index, cursor + 1),
    })

    index = cursor
  }
  return matches
}

function loadCorpusCases() {
  const corpus = readJson(corpusPath, 'syntax corpus manifest')
  assert.equal(corpus.contractVersion, 'm036-s01-syntax-corpus-v1', 'unexpected corpus contract version')
  assert.ok(Array.isArray(corpus.cases) && corpus.cases.length > 0, 'corpus must declare at least one case')

  return corpus.cases.map((caseDef) => {
    const absolutePath = path.join(root, caseDef.path)
    const sourceText = readText(absolutePath, `corpus source for ${caseDef.id}`)
    const snippet = lineSlice(sourceText, caseDef.startLine, caseDef.endLine)

    if (!snippet.trim()) {
      throw new Error(`[m036-s01] corpus case ${caseDef.id} (${caseDef.path}) selected an empty snippet (lines ${caseDef.startLine}-${caseDef.endLine})`)
    }

    const matches = scanInterpolations(snippet, caseDef)
    if (caseDef.expectNoInterpolation) {
      if (matches.length !== 0) {
        throw new Error(`[m036-s01] corpus case ${caseDef.id} (${caseDef.path}) expected no interpolation but found ${matches.map((match) => match.opener).join(', ')}`)
      }
    } else {
      if (!Array.isArray(caseDef.expectedForms) || caseDef.expectedForms.length === 0) {
        throw new Error(`[m036-s01] corpus case ${caseDef.id} (${caseDef.path}) must declare expectedForms or expectNoInterpolation`)
      }
      if (matches.length === 0) {
        throw new Error(`[m036-s01] corpus case ${caseDef.id} (${caseDef.path}) did not contain either interpolation form`)
      }
      const actualForms = [...new Set(matches.map((match) => match.form))].sort()
      const expectedForms = [...new Set(caseDef.expectedForms)].sort()
      assert.deepEqual(actualForms, expectedForms, `[m036-s01] corpus case ${caseDef.id} (${caseDef.path}) drifted from its declared interpolation forms`)
    }

    assert.ok(STRING_SCOPE_BY_KIND[caseDef.expectedStringKind], `[m036-s01] corpus case ${caseDef.id} (${caseDef.path}) has an unsupported expectedStringKind`)

    return {
      ...caseDef,
      absolutePath,
      snippet,
      matches,
    }
  })
}

function loadClusterDecoratorFixture() {
  const absolutePath = clusterDecoratorFixturePath
  const filePath = relativePath(absolutePath)
  const snippet = readText(absolutePath, 'cluster decorator fixture')

  if (!snippet.trim()) {
    throw new Error(`[m036-s01] cluster decorator fixture drift: ${filePath} is empty`)
  }

  const plainDecoratorStart = findRequiredOffset(snippet, '@cluster pub fn add()', 'plain decorator declaration', filePath)
  const countedDecoratorStart = findRequiredOffset(snippet, '@cluster(3) pub fn sync_todos()', 'counted decorator declaration', filePath)
  const bareIdentifierStart = findRequiredOffset(snippet, 'let cluster = 1', 'bare cluster identifier declaration', filePath)

  return {
    absolutePath,
    path: filePath,
    snippet,
    cases: [
      {
        id: 'plain-decorator-at',
        start: plainDecoratorStart,
        end: plainDecoratorStart + 1,
        expectedScopes: [ANNOTATION_PUNCTUATION_SCOPE],
      },
      {
        id: 'plain-decorator-cluster',
        start: plainDecoratorStart + 1,
        end: plainDecoratorStart + '@cluster'.length,
        expectedScopes: [CLUSTER_DECORATOR_SCOPE],
        unexpectedScopes: [VARIABLE_SCOPE],
      },
      {
        id: 'counted-decorator-at',
        start: countedDecoratorStart,
        end: countedDecoratorStart + 1,
        expectedScopes: [ANNOTATION_PUNCTUATION_SCOPE],
      },
      {
        id: 'counted-decorator-cluster',
        start: countedDecoratorStart + 1,
        end: countedDecoratorStart + '@cluster'.length,
        expectedScopes: [CLUSTER_DECORATOR_SCOPE],
        unexpectedScopes: [VARIABLE_SCOPE],
      },
      {
        id: 'counted-decorator-count',
        start: countedDecoratorStart + '@cluster('.length,
        end: countedDecoratorStart + '@cluster(3'.length,
        expectedScopes: [INTEGER_SCOPE],
      },
      {
        id: 'bare-cluster-identifier',
        start: bareIdentifierStart + 'let '.length,
        end: bareIdentifierStart + 'let cluster'.length,
        expectedScopes: [VARIABLE_SCOPE],
        unexpectedScopes: [ANNOTATION_PUNCTUATION_SCOPE, CLUSTER_DECORATOR_SCOPE],
      },
    ],
  }
}

function loadCurrentSyntaxFixture() {
  const absolutePath = currentSyntaxFixturePath
  const filePath = relativePath(absolutePath)
  const snippet = readText(absolutePath, 'current syntax surface fixture')

  const definitions = [
    { id: 'module-doc-comment', search: '##! Current syntax surface fixture.', expectedScopes: [MODULE_DOC_COMMENT_SCOPE] },
    { id: 'doc-comment', search: '## Compiler-derived highlighting probes.', expectedScopes: [DOC_COMMENT_SCOPE] },
    { id: 'line-comment', search: '# This fixture intentionally ends', expectedScopes: [LINE_COMMENT_SCOPE] },
    { id: 'nested-block-comment', search: 'still outer block', expectedScopes: [BLOCK_COMMENT_SCOPE] },
    { id: 'native-decorator-sigil', search: '@native("mesh_u128_add")', token: '@', expectedScopes: [ANNOTATION_PUNCTUATION_SCOPE] },
    { id: 'native-decorator-name', search: '@native("mesh_u128_add")', token: 'native', expectedScopes: [NATIVE_DECORATOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'spaced-native-decorator-name', search: '@ native ("mesh_u128_identity")', token: 'native', expectedScopes: [NATIVE_DECORATOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'spaced-cluster-decorator-name', search: '@ cluster ( 3 )', token: 'cluster', expectedScopes: [CLUSTER_DECORATOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'spaced-cluster-decorator-count', search: '@ cluster ( 3 )', token: '3', expectedScopes: [INTEGER_SCOPE] },
    { id: 'function-declaration-name', search: 'pub fn native_add(', token: 'native_add', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'wide-type-u128', search: 'left :: U128', token: 'U128', expectedScopes: [TYPE_SCOPE] },
    { id: 'from-import-keyword', search: 'from Solana.Read import pubkey', token: 'from', expectedScopePrefixes: [KEYWORD_SCOPE_PREFIX], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'multi-segment-module-path', search: 'from Solana.Read import pubkey', token: 'Solana.Read', expectedScopes: [MODULE_SCOPE] },
    { id: 'unicode-type-declaration', search: 'pub type Δelta = Int', token: 'Δelta', expectedScopes: ['entity.name.type.mesh'], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'struct-declaration-keyword', search: 'pub struct User do', token: 'struct', expectedScopes: ['keyword.declaration.mesh'], unexpectedScopes: ['entity.name.type.mesh'] },
    { id: 'parameterless-interface-signature', search: 'fn ping -> Int', token: 'ping', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'default-interface-method', search: 'fn default_ping -> Int do', token: 'default_ping', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'interface-method-after-multiline-closure', search: 'fn after_multiline_default', token: 'after_multiline_default', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'bare-interface-signature', search: 'fn reset', token: 'reset', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'semicolon-interface-signature', search: 'fn semicolon_reset', token: 'semicolon_reset', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-interface-keyword', search: 'interface #= interface-name trivia =# CommentTriviaHealthProbe', token: 'interface', expectedScopes: ['keyword.declaration.mesh'] },
    { id: 'comment-trivia-interface-name', search: 'CommentTriviaHealthProbe #= interface-do trivia =# do', token: 'CommentTriviaHealthProbe', expectedScopes: [USER_TYPE_SCOPE] },
    { id: 'comment-trivia-interface-method', search: 'fn #= interface-method trivia =# commented_ping', token: 'commented_ping', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-interface-method-comment', search: '#= interface-method trivia =#', expectedScopes: [BLOCK_COMMENT_SCOPE] },
    { id: 'schema-table', search: 'table "users"', token: 'table', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'schema-primary-key', search: 'primary_key :uuid', token: 'primary_key', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'schema-timestamps', search: 'timestamps true', token: 'timestamps', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'schema-timestamps-false', search: 'timestamps false', token: 'timestamps', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'wide-type-u64', search: 'id :: U64', token: 'U64', expectedScopes: [TYPE_SCOPE] },
    { id: 'wide-type-i128', search: 'balance :: I128', token: 'I128', expectedScopes: [TYPE_SCOPE] },
    { id: 'bytes-type', search: 'payload :: Bytes', token: 'Bytes', expectedScopes: [TYPE_SCOPE] },
    { id: 'regex-type', search: 'matcher :: Regex', token: 'Regex', expectedScopes: [TYPE_SCOPE] },
    { id: 'relationship-belongs-to', search: 'belongs_to :account', token: 'belongs_to', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'relationship-has-one', search: 'has_one :profile', token: 'has_one', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'relationship-has-many', search: 'has_many :posts', token: 'has_many', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'deriving-clause', search: 'end deriving(Schema', token: 'deriving', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-schema-table', search: 'table #= table trivia =# "comment_users"', token: 'table', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-schema-primary-key', search: 'primary_key #= primary-key trivia =# :uuid', token: 'primary_key', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-schema-timestamps', search: 'timestamps #= timestamps trivia =# true', token: 'timestamps', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-relationship', search: 'belongs_to #= relationship trivia =# :account', token: 'belongs_to', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-deriving', search: 'deriving #= deriving-call trivia =# (Schema)', token: 'deriving', expectedScopes: [ORM_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-supervisor-name', search: 'CommentTriviaSupervisor #= supervisor-do trivia =# do', token: 'CommentTriviaSupervisor', expectedScopes: [USER_TYPE_SCOPE] },
    { id: 'comment-trivia-supervisor-strategy-key', search: 'strategy #= strategy-colon trivia =# :', token: 'strategy', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-supervisor-strategy-value', search: '#= strategy-value trivia =# one_for_one', token: 'one_for_one', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-supervisor-limit', search: 'max_restarts #= max-restarts trivia =# :', token: 'max_restarts', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-supervisor-child', search: 'child #= child-name trivia =# CommentTriviaWorker', token: 'child', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-supervisor-start', search: 'start #= start-colon trivia =# :', token: 'start', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-nested-declaration', search: 'fn #= declaration-name trivia =# helper = 1', token: 'helper', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-nested-declaration-comment', search: '#= declaration-name trivia =#', expectedScopes: [BLOCK_COMMENT_SCOPE] },
    { id: 'comment-trivia-supervisor-restart-key', search: 'restart #= restart-colon trivia =# :', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-supervisor-restart-value', search: '#= restart-value trivia =# permanent', token: 'permanent', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-supervisor-shutdown-key', search: 'shutdown #= shutdown-colon trivia =# :', token: 'shutdown', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'comment-trivia-supervisor-shutdown-value', search: '#= shutdown-value trivia =# brutal_kill', token: 'brutal_kill', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-strategy-key', search: 'strategy: one_for_all', token: 'strategy', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-strategy-value', search: 'strategy: one_for_all', token: 'one_for_all', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-one-for-one', search: 'strategy: one_for_one', token: 'one_for_one', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-rest-for-one', search: 'strategy: rest_for_one', token: 'rest_for_one', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-simple-one-for-one', search: 'strategy: simple_one_for_one', token: 'simple_one_for_one', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-max-restarts', search: 'max_restarts: 5', token: 'max_restarts', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-max-seconds', search: 'max_seconds: 10', token: 'max_seconds', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-child', search: 'child WorkerPool do', token: 'child', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-start', search: 'start: fn ->', token: 'start', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-restart', search: 'restart: permanent', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-restart-value', search: 'restart: permanent', token: 'permanent', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-transient-value', search: 'restart: transient', token: 'transient', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-temporary-value', search: 'restart: temporary', token: 'temporary', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-shutdown', search: 'shutdown: brutal_kill', token: 'shutdown', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-shutdown-value', search: 'shutdown: brutal_kill', token: 'brutal_kill', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'supervisor-numeric-shutdown', search: 'shutdown: 5000', token: 'shutdown', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'split-do-restart-after-closure', search: 'restart: permanent # split-do-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'wrapped-start-restart-after-closure', search: 'restart: permanent # wrapped-start-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'multiline-wrapped-start-restart-after-closure', search: 'restart: temporary # multiline-wrapped-start-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'wrapped-do-restart-after-closure', search: 'restart: transient # wrapped-do-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'marker-aware-arrow-restart-after-closure', search: 'restart: permanent # marker-aware-arrow-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'marker-aware-do-restart-after-closure', search: 'restart: transient # marker-aware-do-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'multiline-header-restart-after-closure', search: 'restart: temporary # multiline-header-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'lexically-masked-header-restart-after-closure', search: 'restart: permanent # lexically-masked-header-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'trailing-do-closure-restart-after-closure', search: 'restart: transient # trailing-do-closure-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'case-arm-do-closure-restart-after-closure', search: 'restart: temporary # case-arm-do-closure-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'empty-closure-restart-after-closure', search: 'restart: permanent # empty-closure-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'body-lexical-end-restart-after-closure', search: 'restart: transient # body-lexical-end-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'else-if-restart-after-closure', search: 'restart: temporary # else-if-closure-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'multiline-comment-else-if-restart-after-closure', search: 'restart: permanent # multiline-comment-else-if-closure-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'nested-guard-closure-restart-after-closure', search: 'restart: permanent # nested-guard-closure-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'typed-closure-restart-after-closure', search: 'restart: transient # typed-closure-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'guard-block-closure-restart-after-closure', search: 'restart: temporary # guard-block-closure-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'nested-declaration-restart-after-closure', search: 'restart: permanent # nested-declaration-restart', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'semicolon-supervisor-clause', search: 'max_restarts: 13', token: 'max_restarts', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'semicolon-supervisor-limit', search: 'strategy: one_for_one; max_restarts: 3;', token: 'max_restarts', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'semicolon-supervisor-child', search: '; child InlineWorker do', token: 'child', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'semicolon-child-restart', search: 'end; restart: transient;', token: 'restart', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'semicolon-child-restart-value', search: 'end; restart: transient;', token: 'transient', expectedScopes: [SUPERVISOR_VALUE_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'semicolon-child-shutdown', search: '; shutdown: brutal_kill', token: 'shutdown', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'post-child-semicolon-supervisor-limit', search: 'end; max_seconds: 7', token: 'max_seconds', expectedScopes: [SUPERVISOR_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'nested-start-ordinary-strategy-key', search: '        strategy: one_for_one,', token: 'strategy', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'nested-start-ordinary-strategy-value', search: '        strategy: one_for_one,', token: 'one_for_one', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_VALUE_SCOPE] },
    { id: 'nested-start-ordinary-restart-key', search: '        restart: transient,', token: 'restart', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'nested-start-ordinary-shutdown-key', search: '        shutdown: brutal_kill,', token: 'shutdown', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'parameterless-call-handler', search: 'call Get :: Int', token: 'Get', expectedScopes: [FUNCTION_SCOPE] },
    { id: 'parameterless-cast-handler', search: 'cast Reset do', token: 'Reset', expectedScopes: [FUNCTION_SCOPE] },
    { id: 'parameterless-function-declaration', search: 'pub fn heartbeat -> Bool', token: 'heartbeat', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'parameterless-def-declaration', search: 'pub def heartbeat_alias -> Bool', token: 'heartbeat_alias', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'ambiguous-private-return-typed-function-name', search: 'fn private_heartbeat -> Bool', token: 'private_heartbeat', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'private-parameterless-def-declaration', search: 'def private_heartbeat_alias -> Bool', token: 'private_heartbeat_alias', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'ambiguous-private-tuple-result-function-name', search: 'fn private_pair -> (Int, String)', token: 'private_pair', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'ambiguous-guarded-expression-function-name', search: 'fn guarded_tick when ready = ready', token: 'guarded_tick', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'ambiguous-typed-expression-function-name', search: 'fn typed_expression -> Int = 42', token: 'typed_expression', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'ambiguous-guarded-block-declaration-name', search: 'fn guarded_block when ready do', token: 'guarded_block', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'result-type-bang', search: 'User!String', token: '!', expectedScopePrefixes: [OPERATOR_SCOPE_PREFIX] },
    { id: 'orm-module-call-module', search: 'Repo.one(scoped)', token: 'Repo', expectedScopes: [MODULE_SCOPE] },
    { id: 'orm-module-call-function', search: 'Repo.one(scoped)', token: 'one', expectedScopes: [FUNCTION_SCOPE] },
    { id: 'map-literal-opener', search: 'Changeset.cast(%{},', token: '%{', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    { id: 'struct-update-opener', search: '%{user | name:', token: '%{', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    { id: 'struct-update-bar', search: '%{user | name:', token: '|', expectedScopePrefixes: [OPERATOR_SCOPE_PREFIX] },
    { id: 'escaped-regex', search: '~r/users\\/[0-9]+/ims', expectedScopes: [REGEX_SCOPE] },
    { id: 'multiline-regex-opening-line', search: '~r/^first$', expectedScopes: [REGEX_SCOPE] },
    { id: 'multiline-regex-closing-line', search: '^second$/ms', expectedScopes: [REGEX_SCOPE] },
    { id: 'post-multiline-regex-identifier', search: 'let after_multiline_matcher = 1', token: 'after_multiline_matcher', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [REGEX_SCOPE] },
    { id: 'ordering-less-constructor', search: '[Less, Equal, Greater]', token: 'Less', expectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE] },
    { id: 'ordering-equal-constructor', search: '[Less, Equal, Greater]', token: 'Equal', expectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE] },
    { id: 'ordering-greater-constructor', search: '[Less, Equal, Greater]', token: 'Greater', expectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE] },
    { id: 'some-constructor', search: '[Some(1), None, Ok(1), Err("error")]', token: 'Some', expectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE] },
    { id: 'none-constructor', search: '[Some(1), None, Ok(1), Err("error")]', token: 'None', expectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE] },
    { id: 'ok-constructor', search: '[Some(1), None, Ok(1), Err("error")]', token: 'Ok', expectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE] },
    { id: 'err-constructor', search: '[Some(1), None, Ok(1), Err("error")]', token: 'Err', expectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE] },
    { id: 'list-iterator-type', search: '[ListIterator, MapIterator, SetIterator, RangeIterator]', token: 'ListIterator', expectedScopes: [TYPE_SCOPE] },
    { id: 'map-iterator-type', search: '[ListIterator, MapIterator, SetIterator, RangeIterator]', token: 'MapIterator', expectedScopes: [TYPE_SCOPE] },
    { id: 'set-iterator-type', search: '[ListIterator, MapIterator, SetIterator, RangeIterator]', token: 'SetIterator', expectedScopes: [TYPE_SCOPE] },
    { id: 'range-iterator-type', search: '[ListIterator, MapIterator, SetIterator, RangeIterator]', token: 'RangeIterator', expectedScopes: [TYPE_SCOPE] },
    { id: 'decimal-integer-literal', search: 'numeric_literals = [42,', token: '42', expectedScopes: [INTEGER_SCOPE_BY_BASE.decimal] },
    { id: 'hex-integer-literal', search: '0xFF', expectedScopes: [INTEGER_SCOPE_BY_BASE.hex] },
    { id: 'binary-integer-literal', search: '0b1010', expectedScopes: [INTEGER_SCOPE_BY_BASE.binary] },
    { id: 'octal-integer-literal', search: '0o77', expectedScopes: [INTEGER_SCOPE_BY_BASE.octal] },
    { id: 'decimal-float-literal', search: '3.14', expectedScopes: [FLOAT_SCOPE] },
    { id: 'exponent-float-literal', search: '1.0e10', expectedScopes: [FLOAT_SCOPE] },
    { id: 'closure-parameter-not-declaration', search: 'fn value -> value end', token: 'value', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'guarded-closure-parameter-not-declaration', search: 'fn value when value > 0 -> value end', token: 'value', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'line-leading-guarded-closure-parameter-not-declaration', search: '    fn value when value > 0 -> value end,', token: 'value', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'multiline-uppercase-closure-parameter-not-declaration', search: 'fn closure_value -> Int', token: 'closure_value', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'equality-guarded-closure-parameter-not-declaration', search: 'fn equal_value when equal_value == 0 -> equal_value end', token: 'equal_value', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'guarded-do-closure-parameter-not-declaration', search: 'fn guarded_do_value when guarded_do_value > 0 do', token: 'guarded_do_value', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'constructor-pattern-closure-not-declaration', search: 'fn Some(constructor_value) -> constructor_value end', token: 'Some', expectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'string-equals-closure-parameter-not-declaration', search: 'fn option_value -> Some("a=b") end', token: 'option_value', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'trailing-do-closure-parameter-not-declaration', search: 'fn builder_value -> Builder.make() do', token: 'builder_value', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [FUNCTION_SCOPE] },
    { id: 'private-parameterized-function-declaration', search: 'fn contextual_names(native, table', token: 'contextual_names', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'ordinary-strategy-key', search: 'configure(strategy: one_for_one', token: 'strategy', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'ordinary-strategy-value', search: 'configure(strategy: one_for_one', token: 'one_for_one', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_VALUE_SCOPE] },
    { id: 'ordinary-restart-key', search: 'restart: transient, shutdown:', token: 'restart', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'ordinary-restart-value', search: 'restart: transient, shutdown:', token: 'transient', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_VALUE_SCOPE] },
    { id: 'ordinary-shutdown-key', search: 'shutdown: brutal_kill)', token: 'shutdown', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'ordinary-shutdown-value', search: 'shutdown: brutal_kill)', token: 'brutal_kill', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_VALUE_SCOPE] },
    { id: 'multiline-ordinary-strategy-key', search: '    strategy: one_for_one,', token: 'strategy', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'multiline-ordinary-strategy-value', search: '    strategy: one_for_one,', token: 'one_for_one', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_VALUE_SCOPE] },
    { id: 'multiline-ordinary-restart-key', search: '    restart: transient,', token: 'restart', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'multiline-ordinary-restart-value', search: '    restart: transient,', token: 'transient', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_VALUE_SCOPE] },
    { id: 'multiline-ordinary-shutdown-key', search: '    shutdown: brutal_kill,', token: 'shutdown', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'multiline-ordinary-shutdown-value', search: '    shutdown: brutal_kill,', token: 'brutal_kill', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_VALUE_SCOPE] },
    { id: 'ordinary-deriving-call', search: 'deriving(value)', token: 'deriving', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [ORM_SCOPE] },
    { id: 'or-pattern-bar', search: 'Ok(value) | Some(value)', token: '|', expectedScopePrefixes: [OPERATOR_SCOPE_PREFIX] },
    { id: 'as-pattern', search: 'None as missing', token: 'as', expectedScopePrefixes: [KEYWORD_SCOPE_PREFIX], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'wildcard-pattern', search: '_ -> updated', token: '_', expectedScopes: [WILDCARD_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'unicode-identifier', search: 'let κόσμος = pubkey()', token: 'κόσμος', expectedScopes: [VARIABLE_SCOPE] },
    { id: 'unicode-letter-number-identifier', search: 'let Ⅳalue = 4', token: 'Ⅳalue', expectedScopes: [VARIABLE_SCOPE] },
    { id: 'unicode-uncased-identifier', search: 'let 四季 = 4', token: '四季', expectedScopes: [VARIABLE_SCOPE] },
    { id: 'bare-function-call', search: 'println("#{κόσμος}")', token: 'println', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [VARIABLE_SCOPE] },
    { id: 'interpolated-identifier', search: 'println("#{κόσμος}")', token: 'κόσμος', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [USER_TYPE_SCOPE, FUNCTION_SCOPE] },
    { id: 'unary-bang', search: '!false', token: '!', expectedScopePrefixes: [OPERATOR_SCOPE_PREFIX] },
    { id: 'valid-lowercase-atom', search: ':account', expectedScopes: [ATOM_SCOPE] },
    { id: 'plain-native-identifier', search: 'fn contextual_names(native,', token: 'native', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [NATIVE_DECORATOR_SCOPE] },
    { id: 'plain-table-identifier', search: 'native, table, strategy', token: 'table', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [ORM_SCOPE] },
    { id: 'plain-strategy-identifier', search: 'table, strategy, deriving', token: 'strategy', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [SUPERVISOR_SCOPE] },
    { id: 'plain-deriving-identifier', search: 'strategy, deriving, from', token: 'deriving', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [ORM_SCOPE] },
    { id: 'plain-from-identifier', search: 'deriving, from, as', token: 'from', expectedScopes: [VARIABLE_SCOPE] },
    { id: 'plain-as-identifier', search: 'from, as) do', token: 'as', expectedScopes: [VARIABLE_SCOPE] },
    { id: 'unicode-keyword-prefix-identifier', search: 'unicode_boundary_names(letπ,', token: 'letπ', expectedScopes: [VARIABLE_SCOPE], unexpectedScopePrefixes: [KEYWORD_SCOPE_PREFIX] },
    { id: 'unicode-builtin-prefix-identifier', search: 'letπ, Inté, Someδ, trueλ', token: 'Inté', expectedScopes: [USER_TYPE_SCOPE], unexpectedScopes: [TYPE_SCOPE] },
    { id: 'unicode-constructor-prefix-identifier', search: 'letπ, Inté, Someδ, trueλ', token: 'Someδ', expectedScopes: [USER_TYPE_SCOPE], unexpectedScopes: [BUILTIN_CONSTRUCTOR_SCOPE] },
    { id: 'unicode-constant-prefix-identifier', search: 'letπ, Inté, Someδ, trueλ', token: 'trueλ', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [LANGUAGE_CONSTANT_SCOPE] },
    { id: 'unicode-cluster-prefix-identifier', search: '@clusterπ', token: 'clusterπ', expectedScopes: [VARIABLE_SCOPE], unexpectedScopes: [CLUSTER_DECORATOR_SCOPE] },
    { id: 'unicode-native-prefix-identifier', search: '@nativeπ("symbol")', token: 'nativeπ', expectedScopes: [FUNCTION_SCOPE], unexpectedScopes: [NATIVE_DECORATOR_SCOPE] },
  ]

  return {
    absolutePath,
    path: filePath,
    snippet,
    cases: definitions.map((definition) => scopeCase(snippet, filePath, definition)),
  }
}

function tokenizeSnippet(grammar, code) {
  const normalized = code.replace(/\r\n/g, '\n')
  const lines = normalized.split('\n')
  const segments = []
  let lineOffset = 0
  let state = null

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
    const line = lines[lineIndex]
    const result = grammar.tokenizeLine(line, state)
    state = result.ruleStack

    for (const token of result.tokens) {
      segments.push({
        line: lineIndex + 1,
        start: lineOffset + token.startIndex,
        end: lineOffset + token.endIndex,
        scopes: token.scopes,
        text: line.slice(token.startIndex, token.endIndex),
      })
    }

    lineOffset += line.length
    if (lineIndex < lines.length - 1) lineOffset += 1
  }

  return segments
}

function scopesForRange(segments, start, end) {
  const scopes = new Set()
  for (const segment of segments) {
    if (segment.start < end && start < segment.end) {
      for (const scope of segment.scopes) scopes.add(scope)
    }
  }
  return scopes
}

function scopeCoversRange(segments, start, end, predicate) {
  let cursor = start
  for (const segment of segments) {
    const overlapStart = Math.max(start, segment.start)
    const overlapEnd = Math.min(end, segment.end)
    if (overlapEnd <= cursor) continue
    if (overlapStart > cursor || !predicate(segment.scopes)) return false
    cursor = overlapEnd
    if (cursor >= end) return true
  }
  return false
}

function scopesToSignature(segments) {
  return segments.map((segment) => ({
    start: segment.start,
    end: segment.end,
    scopes: segment.scopes,
  }))
}

function describeScopes(scopes) {
  return [...scopes].sort().join(', ') || '(none)'
}

function assertScopeContract(engineName, fixture, segments, caseDef) {
  const actualScopes = scopesForRange(segments, caseDef.start, caseDef.end)
  const range = formatRange(fixture.snippet, caseDef.start, caseDef.end)

  for (const scope of caseDef.expectedScopes ?? []) {
    assert.ok(
      scopeCoversRange(segments, caseDef.start, caseDef.end, (scopes) => scopes.includes(scope)),
      `[m036-s01] shared-surface syntax drift detected: engine=${engineName} file=${fixture.path} case=${caseDef.id} range=${range} issue=${scope} does not cover full token actual=${describeScopes(actualScopes)}`,
    )
  }

  for (const scope of caseDef.unexpectedScopes ?? []) {
    assert.ok(
      !actualScopes.has(scope),
      `[m036-s01] shared-surface syntax drift detected: engine=${engineName} file=${fixture.path} case=${caseDef.id} range=${range} issue=unexpected ${scope} actual=${describeScopes(actualScopes)}`,
    )
  }

  for (const prefix of caseDef.expectedScopePrefixes ?? []) {
    assert.ok(
      scopeCoversRange(segments, caseDef.start, caseDef.end, (scopes) => scopes.some((scope) => scope.startsWith(prefix))),
      `[m036-s01] shared-surface syntax drift detected: engine=${engineName} file=${fixture.path} case=${caseDef.id} range=${range} issue=scope prefix ${prefix} does not cover full token actual=${describeScopes(actualScopes)}`,
    )
  }

  for (const prefix of caseDef.unexpectedScopePrefixes ?? []) {
    assert.ok(
      ![...actualScopes].some((scope) => scope.startsWith(prefix)),
      `[m036-s01] shared-surface syntax drift detected: engine=${engineName} file=${fixture.path} case=${caseDef.id} range=${range} issue=unexpected scope prefix ${prefix} actual=${describeScopes(actualScopes)}`,
    )
  }
}

function tokenKindVariants(source, sectionName, nextSectionName) {
  const startMarker = `// ── ${sectionName}`
  const endMarker = `// ── ${nextSectionName}`
  const start = source.indexOf(startMarker)
  const end = source.indexOf(endMarker, start + startMarker.length)
  assert.ok(start >= 0 && end > start, `compiler token sections ${sectionName}/${nextSectionName} must remain discoverable`)
  return [...source.slice(start, end).matchAll(/^\s{4}([A-Z][A-Za-z0-9_]*)(?:\([^)]*\))?,/gm)].map((match) => match[1])
}

function compilerKeywords(source) {
  const start = source.indexOf('pub fn keyword_from_str')
  const end = source.indexOf('#[cfg(test)]', start)
  assert.ok(start >= 0 && end > start, 'compiler keyword_from_str implementation must remain discoverable')
  return [...source.slice(start, end).matchAll(/^\s*"([^"]+)"\s*=>\s*Some\(TokenKind::/gm)].map((match) => match[1])
}

function assertTokenProbe(engineName, tokenize, probe) {
  const snippet = probe.code ?? `left ${probe.token} right`
  const start = probe.start ?? snippet.indexOf(probe.token)
  assert.ok(start >= 0, `probe ${probe.id} token must occur in its snippet`)
  const fixture = {
    path: `${relativePath(compilerTokenPath)}#${probe.id}`,
    snippet,
  }
  assertScopeContract(engineName, fixture, tokenize(snippet), {
    ...probe,
    start,
    end: start + probe.token.length,
  })
}

function verifyContract(engineName, corpusCase, segments) {
  const drifts = []
  const expectedStringScope = STRING_SCOPE_BY_KIND[corpusCase.expectedStringKind]
  const overallStringScopes = scopesForRange(segments, 0, corpusCase.snippet.length)
  if (!overallStringScopes.has(expectedStringScope)) {
    drifts.push({
      engine: engineName,
      file: corpusCase.path,
      caseId: corpusCase.id,
      form: corpusCase.expectNoInterpolation ? 'none' : corpusCase.expectedForms.join('+'),
      issue: `missing ${expectedStringScope} anywhere in the snippet`,
      actualScopes: describeScopes(overallStringScopes),
    })
  }

  if (corpusCase.expectNoInterpolation) {
    const noInterpolationScopes = [BEGIN_SCOPE, META_SCOPE, END_SCOPE].filter((scope) => overallStringScopes.has(scope))
    if (noInterpolationScopes.length > 0) {
      drifts.push({
        engine: engineName,
        file: corpusCase.path,
        caseId: corpusCase.id,
        form: 'none',
        issue: `unexpected interpolation scopes in a plain string`,
        actualScopes: noInterpolationScopes.join(', '),
      })
    }
    return drifts
  }

  for (const match of corpusCase.matches) {
    const startScopes = scopesForRange(segments, match.start, match.openEnd)
    if (!startScopes.has(BEGIN_SCOPE)) {
      drifts.push({
        engine: engineName,
        file: corpusCase.path,
        caseId: corpusCase.id,
        form: match.form,
        issue: `missing ${BEGIN_SCOPE} for ${JSON.stringify(match.opener)}`,
        actualScopes: describeScopes(startScopes),
      })
    }

    const expressionScopes = scopesForRange(segments, match.exprStart, Math.max(match.exprStart + 1, match.exprEnd))
    if (!expressionScopes.has(META_SCOPE)) {
      drifts.push({
        engine: engineName,
        file: corpusCase.path,
        caseId: corpusCase.id,
        form: match.form,
        issue: `missing ${META_SCOPE} for expression ${JSON.stringify(match.expression)}`,
        actualScopes: describeScopes(expressionScopes),
      })
    }

    const endScopes = scopesForRange(segments, match.endStart, match.endEnd)
    if (!endScopes.has(END_SCOPE)) {
      drifts.push({
        engine: engineName,
        file: corpusCase.path,
        caseId: corpusCase.id,
        form: match.form,
        issue: `missing ${END_SCOPE} for ${JSON.stringify(match.text)}`,
        actualScopes: describeScopes(endScopes),
      })
    }
  }

  return drifts
}

function formatDrifts(drifts) {
  return [
    '[m036-s01] shared-surface syntax drift detected:',
    ...drifts.map((drift) => `- engine=${drift.engine} file=${drift.file} case=${drift.caseId} form=${drift.form} issue=${drift.issue} actual=${drift.actualScopes}`),
  ].join('\n')
}

async function createTextMateHarness(options = {}) {
  const registryModulePath = options.registryModulePath ?? 'website/node_modules/@shikijs/vscode-textmate/dist/index.js'
  const engineModulePath = options.engineModulePath ?? 'website/node_modules/@shikijs/engine-javascript/dist/index.mjs'
  const grammarPath = options.grammarPath ?? relativePath(sharedGrammarPath)
  const [{ Registry }, { createJavaScriptRegexEngine }] = await Promise.all([
    importRepoModule(registryModulePath, 'TextMate dependency'),
    importRepoModule(engineModulePath, 'TextMate regex engine dependency'),
  ])

  const grammar = readJson(path.join(root, grammarPath), 'shared grammar')
  const regexEngine = createJavaScriptRegexEngine()
  const registry = new Registry({
    onigLib: {
      createOnigScanner(patterns) {
        return regexEngine.createScanner(patterns)
      },
      createOnigString(text) {
        return regexEngine.createString(text)
      },
    },
    loadGrammar(scopeName) {
      return scopeName === grammar.scopeName ? grammar : null
    },
  })

  const loadedGrammar = registry.loadGrammar(grammar.scopeName)
  if (!loadedGrammar) {
    throw new Error(`[m036-s01] failed to load textmate grammar from ${grammarPath}`)
  }

  return {
    tokenize(code) {
      return tokenizeSnippet(loadedGrammar, code)
    },
  }
}

async function createShikiHarness(options = {}) {
  const shikiModulePath = options.shikiModulePath ?? 'website/node_modules/shiki/dist/index.mjs'
  const grammarPath = options.grammarPath ?? relativePath(sharedGrammarPath)
  const [shikiModule] = await Promise.all([
    importRepoModule(shikiModulePath, 'Shiki dependency'),
  ])

  const grammar = readJson(path.join(root, grammarPath), 'shared grammar')
  const meshLight = readJson(shikiLightThemePath, 'mesh light theme')
  const meshDark = readJson(shikiDarkThemePath, 'mesh dark theme')

  const highlighter = await withTimeout('shiki highlighter load', 5000, () =>
    shikiModule.createHighlighter({
      themes: [meshLight, meshDark],
      langs: [{ ...grammar, name: 'mesh' }],
    }),
  )

  const loadedGrammar = highlighter.getLanguage('mesh')
  if (!loadedGrammar || typeof loadedGrammar.tokenizeLine !== 'function') {
    throw new Error(`[m036-s01] failed to resolve the docs-side shiki grammar for mesh from ${grammarPath}`)
  }

  return {
    render(code) {
      return highlighter.codeToHtml(code, {
        lang: 'mesh',
        themes: { light: 'mesh-light', dark: 'mesh-dark' },
        defaultColor: false,
      })
    },
    tokenize(code) {
      return tokenizeSnippet(loadedGrammar, code)
    },
    dispose() {
      highlighter.dispose()
    },
  }
}

test('corpus manifest resolves audited repo snippets and keeps cases named', () => {
  const cases = loadCorpusCases()
  assert.ok(cases.length >= 10, 'expected a non-toy syntax corpus')
  for (const corpusCase of cases) {
    assert.ok(corpusCase.id, 'case ids must be present')
    assert.ok(corpusCase.path, `case ${corpusCase.id} must carry its source path`)
    assert.ok(corpusCase.snippet.includes('"'), `case ${corpusCase.id} should resolve string-bearing source text`)
  }
})

test('verifier helpers fail closed for malformed corpus entries and broken loader paths', async () => {
  const partialCoverage = [
    { start: 0, end: 1, scopes: ['keyword.operator.mesh'] },
    { start: 1, end: 2, scopes: ['source.mesh'] },
  ]
  assert.equal(
    scopeCoversRange(partialCoverage, 0, 2, (scopes) => scopes.includes('keyword.operator.mesh')),
    false,
    'a partially scoped token must not satisfy a full-token contract',
  )

  const missingSourcePath = path.join(root, 'scripts/fixtures/m036-s01/missing.mpl')
  assert.throws(() => readText(missingSourcePath, 'corpus source for missing-source'), /missing corpus source for missing-source: scripts\/fixtures\/m036-s01\/missing\.mpl/)

  const emptySelectionCase = {
    id: 'empty-selection',
    path: 'tests/fixtures/interpolation.mpl',
    startLine: 99,
    endLine: 99,
    expectedForms: ['dollar'],
    expectedStringKind: 'double',
  }
  const interpolationFixture = readText(path.join(root, emptySelectionCase.path), 'test interpolation fixture')
  assert.throws(
    () => {
      const snippet = lineSlice(interpolationFixture, emptySelectionCase.startLine, emptySelectionCase.endLine)
      if (!snippet.trim()) {
        throw new Error(`[m036-s01] corpus case ${emptySelectionCase.id} (${emptySelectionCase.path}) selected an empty snippet (lines ${emptySelectionCase.startLine}-${emptySelectionCase.endLine})`)
      }
    },
    /corpus case empty-selection \(tests\/fixtures\/interpolation\.mpl\) selected an empty snippet/,
  )

  const malformedCase = {
    id: 'missing-form-contract',
    path: 'tests/fixtures/interpolation.mpl',
    startLine: 4,
    endLine: 4,
    expectedStringKind: 'double',
  }
  const malformedSnippet = lineSlice(interpolationFixture, malformedCase.startLine, malformedCase.endLine)
  assert.throws(
    () => {
      const matches = scanInterpolations(malformedSnippet, malformedCase)
      if (!Array.isArray(malformedCase.expectedForms) || malformedCase.expectedForms.length === 0) {
        throw new Error(`[m036-s01] corpus case ${malformedCase.id} (${malformedCase.path}) must declare expectedForms or expectNoInterpolation`)
      }
      return matches
    },
    /corpus case missing-form-contract \(tests\/fixtures\/interpolation\.mpl\) must declare expectedForms or expectNoInterpolation/,
  )

  await assert.rejects(
    () => createTextMateHarness({ registryModulePath: 'website/node_modules/@shikijs/vscode-textmate/dist/does-not-exist.js' }),
    /missing TextMate dependency: website\/node_modules\/@shikijs\/vscode-textmate\/dist\/does-not-exist\.js/,
  )

  await assert.rejects(
    () => createShikiHarness({ grammarPath: 'tools/editors/vscode-mesh/syntaxes/does-not-exist.json' }),
    /missing shared grammar: tools\/editors\/vscode-mesh\/syntaxes\/does-not-exist\.json/,
  )

  await assert.rejects(
    () => withTimeout('shiki stalled engine', 25, () => new Promise(() => {})),
    /shiki stalled engine timed out after 25ms/,
  )
})

test('shared grammar matches the audited interpolation contract in both TextMate and Shiki', async () => {
  const corpusCases = loadCorpusCases()
  const [textmate, shiki] = await Promise.all([createTextMateHarness(), createShikiHarness()])
  const drifts = []

  try {
    for (const corpusCase of corpusCases) {
      const textmateTokens = textmate.tokenize(corpusCase.snippet)
      const shikiTokens = shiki.tokenize(corpusCase.snippet)
      const rendered = shiki.render(corpusCase.snippet)

      assert.match(rendered, /<pre class="shiki /, `[m036-s01] shiki render output drifted for ${corpusCase.id}`)

      drifts.push(...verifyContract('textmate', corpusCase, textmateTokens))
      drifts.push(...verifyContract('shiki', corpusCase, shikiTokens))

      const textmateSignature = scopesToSignature(textmateTokens)
      const shikiSignature = scopesToSignature(shikiTokens)
      if (JSON.stringify(textmateSignature) !== JSON.stringify(shikiSignature)) {
        drifts.push({
          engine: 'textmate',
          file: corpusCase.path,
          caseId: corpusCase.id,
          form: corpusCase.expectNoInterpolation ? 'none' : corpusCase.expectedForms.join('+'),
          issue: 'token signature diverged from shiki',
          actualScopes: JSON.stringify(textmateSignature),
        })
        drifts.push({
          engine: 'shiki',
          file: corpusCase.path,
          caseId: corpusCase.id,
          form: corpusCase.expectNoInterpolation ? 'none' : corpusCase.expectedForms.join('+'),
          issue: 'token signature diverged from textmate',
          actualScopes: JSON.stringify(shikiSignature),
        })
      }
    }
  } finally {
    shiki.dispose()
  }

  assert.equal(drifts.length, 0, formatDrifts(drifts))
})

test('shared grammar scopes @cluster decorators consistently in both TextMate and Shiki', async () => {
  const fixture = loadClusterDecoratorFixture()
  const [textmate, shiki] = await Promise.all([createTextMateHarness(), createShikiHarness()])

  try {
    const textmateTokens = textmate.tokenize(fixture.snippet)
    const shikiTokens = shiki.tokenize(fixture.snippet)
    const rendered = shiki.render(fixture.snippet)

    assert.match(rendered, /<pre class="shiki /, `[m036-s01] shiki render output drifted for ${fixture.path}`)

    const textmateSignature = scopesToSignature(textmateTokens)
    const shikiSignature = scopesToSignature(shikiTokens)
    assert.equal(
      JSON.stringify(textmateSignature),
      JSON.stringify(shikiSignature),
      `[m036-s01] shared-surface syntax drift detected: engine=both file=${fixture.path} case=cluster-decorator-signature issue=token signature diverged actual=textmate=${JSON.stringify(textmateSignature)} shiki=${JSON.stringify(shikiSignature)}`,
    )

    for (const caseDef of fixture.cases) {
      assertScopeContract('textmate', fixture, textmateTokens, caseDef)
      assertScopeContract('shiki', fixture, shikiTokens, caseDef)
    }
  } finally {
    shiki.dispose()
  }
})

test('shared grammar covers the complete compiler token surface and current language DSLs', async () => {
  const fixture = loadCurrentSyntaxFixture()
  const compilerTokens = readText(compilerTokenPath, 'compiler token vocabulary')
  const [textmate, shiki] = await Promise.all([createTextMateHarness(), createShikiHarness()])

  const operatorProbes = {
    Plus: { token: '+', expectedScopes: ['keyword.operator.arithmetic.mesh'] },
    Minus: { token: '-', expectedScopes: ['keyword.operator.arithmetic.mesh'] },
    Star: { token: '*', expectedScopes: ['keyword.operator.arithmetic.mesh'] },
    Slash: { token: '/', expectedScopes: ['keyword.operator.arithmetic.mesh'] },
    Percent: { token: '%', expectedScopes: ['keyword.operator.arithmetic.mesh'] },
    EqEq: { token: '==', expectedScopes: ['keyword.operator.comparison.mesh'] },
    NotEq: { token: '!=', expectedScopes: ['keyword.operator.comparison.mesh'] },
    Lt: { token: '<', expectedScopes: ['keyword.operator.comparison.mesh'] },
    Gt: { token: '>', expectedScopes: ['keyword.operator.comparison.mesh'] },
    LtEq: { token: '<=', expectedScopes: ['keyword.operator.comparison.mesh'] },
    GtEq: { token: '>=', expectedScopes: ['keyword.operator.comparison.mesh'] },
    AmpAmp: { token: '&&', expectedScopes: ['keyword.operator.logical.and.mesh'] },
    PipePipe: { token: '||', expectedScopes: ['keyword.operator.logical.or.mesh'] },
    Bang: { token: '!', expectedScopes: ['keyword.operator.result.mesh'] },
    Pipe: { token: '|>', expectedScopes: ['keyword.operator.pipe.mesh'] },
    SlotPipe: { token: '|2>', expectedScopes: ['keyword.operator.pipe.slot.mesh'] },
    DotDot: { token: '..', expectedScopes: ['keyword.operator.range.mesh'] },
    Diamond: { token: '<>', expectedScopes: ['keyword.operator.diamond.mesh'] },
    PlusPlus: { token: '++', expectedScopes: ['keyword.operator.concat.mesh'] },
    Eq: { token: '=', expectedScopes: ['keyword.operator.assignment.mesh'] },
    Arrow: { token: '->', expectedScopes: ['keyword.operator.arrow.mesh'] },
    FatArrow: { token: '=>', expectedScopes: ['keyword.operator.arrow.fat.mesh'] },
    ColonColon: { token: '::', expectedScopes: ['keyword.operator.annotation.mesh'] },
    Question: { token: '?', expectedScopes: ['keyword.operator.try.mesh'] },
    Bar: { token: '|', expectedScopes: ['keyword.operator.pattern.mesh'] },
  }
  assert.deepEqual(
    Object.keys(operatorProbes).sort(),
    tokenKindVariants(compilerTokens, 'Operators', 'Delimiters').sort(),
    'operator probes must change when the compiler token vocabulary changes',
  )

  const delimiterProbes = {
    LParen: { token: '(', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    RParen: { token: ')', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    LBracket: { token: '[', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    RBracket: { token: ']', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    LBrace: { token: '{', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    RBrace: { token: '}', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
  }
  assert.deepEqual(
    Object.keys(delimiterProbes).sort(),
    tokenKindVariants(compilerTokens, 'Delimiters', 'Punctuation').sort(),
    'delimiter probes must change when the compiler token vocabulary changes',
  )

  const punctuationProbes = {
    Comma: { token: ',', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    Dot: { token: '.', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    Colon: { token: ':', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    Semicolon: { token: ';', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
    At: { token: '@', expectedScopePrefixes: [PUNCTUATION_SCOPE_PREFIX] },
  }
  assert.deepEqual(
    [...Object.keys(punctuationProbes), 'Newline'].sort(),
    tokenKindVariants(compilerTokens, 'Punctuation', 'Literals').sort(),
    'punctuation probes must change when the compiler token vocabulary changes',
  )

  assert.deepEqual(
    tokenKindVariants(compilerTokens, 'Literals', 'Identifiers and comments').sort(),
    ['Atom', 'FloatLiteral', 'IntLiteral', 'InterpolationEnd', 'InterpolationStart', 'RegexLiteral', 'StringContent', 'StringEnd', 'StringStart'].sort(),
    'literal coverage must change when the compiler token vocabulary changes',
  )
  assert.deepEqual(
    tokenKindVariants(compilerTokens, 'Identifiers and comments', 'Special').sort(),
    ['Comment', 'DocComment', 'Ident', 'ModuleDocComment'].sort(),
    'identifier/comment coverage must change when the compiler token vocabulary changes',
  )

  const builtInTypes = [
    'Atom',
    'Bool',
    'BootstrapStatus',
    'Bytes',
    'ContinuityAuthorityStatus',
    'ContinuityRecord',
    'ContinuitySubmitDecision',
    'DateTime',
    'Float',
    'Fun',
    'HttpClientMetrics',
    'HttpResponse',
    'I128',
    'Int',
    'Json',
    'List',
    'ListIterator',
    'Map',
    'MapIterator',
    'Never',
    'Option',
    'Ordering',
    'PgConn',
    'Pid',
    'PoolHandle',
    'Queue',
    'Range',
    'RangeIterator',
    'Regex',
    'Request',
    'Response',
    'Result',
    'Router',
    'Set',
    'SetIterator',
    'SqliteConn',
    'String',
    'Tuple',
    'U128',
    'U64',
    'Unit',
    'WsMessage',
  ]

  const negativeProbes = [
    { id: 'invalid-uppercase-atom', code: ':Invalid', token: ':Invalid', unexpectedScopes: [ATOM_SCOPE] },
    { id: 'missing-regex-delimiter', code: '~r', token: '~r', unexpectedScopes: [REGEX_SCOPE] },
    { id: 'invalid-regex-flag', code: '~r/value/z', token: '~r/value/z', unexpectedScopes: [REGEX_SCOPE] },
    { id: 'invalid-slot-pipe-zero', code: 'left |0> right', token: '|0>', unexpectedScopePrefixes: [OPERATOR_SCOPE_PREFIX] },
    { id: 'invalid-slot-pipe-one', code: 'left |1> right', token: '|1>', unexpectedScopePrefixes: [OPERATOR_SCOPE_PREFIX] },
    { id: 'incomplete-slot-pipe', code: 'left |2 right', token: '|2', unexpectedScopePrefixes: [OPERATOR_SCOPE_PREFIX] },
  ]

  try {
    const textmateTokens = textmate.tokenize(fixture.snippet)
    const shikiTokens = shiki.tokenize(fixture.snippet)
    assert.deepEqual(
      scopesToSignature(textmateTokens),
      scopesToSignature(shikiTokens),
      `[m036-s01] current syntax fixture diverged between TextMate and Shiki: ${fixture.path}`,
    )

    for (const caseDef of fixture.cases) {
      assertScopeContract('textmate', fixture, textmateTokens, caseDef)
      assertScopeContract('shiki', fixture, shikiTokens, caseDef)
    }

    for (const keyword of compilerKeywords(compilerTokens)) {
      const expectedScopePrefixes = ['true', 'false', 'nil'].includes(keyword)
        ? ['constant.language.']
        : [KEYWORD_SCOPE_PREFIX]
      const probe = { id: `keyword-${keyword}`, code: keyword, token: keyword, expectedScopePrefixes }
      assertTokenProbe('textmate', textmate.tokenize, probe)
      assertTokenProbe('shiki', shiki.tokenize, probe)
    }

    for (const [id, probe] of Object.entries({ ...operatorProbes, ...delimiterProbes, ...punctuationProbes })) {
      assertTokenProbe('textmate', textmate.tokenize, { id, ...probe })
      assertTokenProbe('shiki', shiki.tokenize, { id, ...probe })
    }

    for (const type of builtInTypes) {
      const probe = { id: `builtin-type-${type}`, code: type, token: type, expectedScopes: [TYPE_SCOPE] }
      assertTokenProbe('textmate', textmate.tokenize, probe)
      assertTokenProbe('shiki', shiki.tokenize, probe)
    }

    for (const probe of negativeProbes) {
      assertTokenProbe('textmate', textmate.tokenize, probe)
      assertTokenProbe('shiki', shiki.tokenize, probe)
    }

    for (const [themePath, themeName] of [[shikiLightThemePath, 'mesh-light'], [shikiDarkThemePath, 'mesh-dark']]) {
      const theme = readJson(themePath, `${themeName} theme`)
      const themeScopes = new Set(theme.tokenColors.flatMap((rule) => {
        if (Array.isArray(rule.scope)) return rule.scope
        return rule.scope ? [rule.scope] : []
      }))
      for (const scope of ['invalid.illegal', 'keyword.operator', 'keyword.other', 'storage.modifier.annotation']) {
        assert.ok(
          themeScopes.has(scope),
          `[m036-s01] ${themeName} must style ${scope} instead of leaving current Mesh syntax at the default foreground`,
        )
      }
    }
  } finally {
    shiki.dispose()
  }
})
