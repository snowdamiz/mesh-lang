local repo_root = assert(os.getenv('MESH_REPO_ROOT'), 'MESH_REPO_ROOT is required')
local requested_phase = vim.g.mesh_smoke_phase or os.getenv('MESH_NVIM_SMOKE_PHASE') or 'all'
local uv = vim.uv or vim.loop
local mesh = require('mesh')
local example_root = vim.fs.joinpath(repo_root, 'examples', 'todo-postgres')
local example_health_path = vim.fs.joinpath(example_root, 'api', 'health.mpl')
local example_todos_path = vim.fs.joinpath(example_root, 'api', 'todos.mpl')

local run_syntax = requested_phase == 'all' or requested_phase == 'syntax'
local run_lsp = requested_phase == 'all' or requested_phase == 'lsp'

local function canonical(path)
  if not path or path == '' then
    return nil
  end
  local real = uv.fs_realpath(path)
  return vim.fs.normalize(real or path)
end

local function rel(path)
  if not path or path == '' then
    return '<none>'
  end
  local normalized = canonical(path) or vim.fs.normalize(path)
  local repo = canonical(repo_root) or vim.fs.normalize(repo_root)
  if normalized:sub(1, #repo) == repo then
    local suffix = normalized:sub(#repo + 1)
    if suffix:sub(1, 1) == '/' then
      suffix = suffix:sub(2)
    end
    return suffix ~= '' and suffix or '.'
  end
  return normalized
end

local function fail(phase, message)
  io.stderr:write(string.format('[m036-s02] phase=%s result=fail %s\n', phase, message))
  if phase == 'lsp' then
    local last_error = vim.g.mesh_lsp_last_error
    if type(last_error) == 'string' and last_error ~= '' then
      io.stderr:write(string.format('[m036-s02] phase=lsp last_error=%s\n', last_error:gsub('\n', ' | ')))
    end

    local last_resolution = vim.g.mesh_lsp_last_resolution
    if type(last_resolution) == 'table' then
      local ok, encoded = pcall(vim.json.encode, last_resolution)
      if ok and type(encoded) == 'string' and encoded ~= '' then
        io.stderr:write(string.format('[m036-s02] phase=lsp last_resolution=%s\n', encoded))
      end
    end
  end
  vim.cmd('cquit 1')
end

local function read_file(path)
  local lines = vim.fn.readfile(path)
  return table.concat(lines, '\n')
end

local function decode_json_file(path)
  return vim.json.decode(read_file(path))
end

local function open_buffer(path)
  vim.cmd('silent! %bwipeout!')
  vim.cmd('edit ' .. vim.fn.fnameescape(path))
  return vim.api.nvim_get_current_buf()
end

local function ensure_clean_dir(path)
  vim.fn.delete(path, 'rf')
  if vim.fn.mkdir(path, 'p') ~= 1 then
    fail('lsp', string.format('reason=failed_to_create_dir path=%s', path))
  end
end

local function write_file(path, lines)
  local parent = vim.fs.dirname(path)
  if parent and parent ~= '' and vim.fn.mkdir(parent, 'p') ~= 1 then
    fail('lsp', string.format('reason=failed_to_create_parent path=%s', parent))
  end
  vim.fn.writefile(lines, path)
end

local function syntax_info(line, col)
  local stack_ids = vim.fn.synstack(line, col)
  local stack_names = {}
  for _, id in ipairs(stack_ids) do
    local name = vim.fn.synIDattr(id, 'name')
    if name ~= nil and name ~= '' then
      table.insert(stack_names, name)
    end
  end

  local direct_id = vim.fn.synID(line, col, 1)
  local direct_name = vim.fn.synIDattr(direct_id, 'name')
  local translated_name = vim.fn.synIDattr(vim.fn.synIDtrans(direct_id), 'name')

  return {
    stack = stack_names,
    direct = direct_name,
    translated = translated_name,
  }
end

local function names_text(info)
  local parts = {}
  if #info.stack > 0 then
    table.insert(parts, 'stack=' .. table.concat(info.stack, '>'))
  end
  if info.direct and info.direct ~= '' then
    table.insert(parts, 'direct=' .. info.direct)
  end
  if info.translated and info.translated ~= '' then
    table.insert(parts, 'translated=' .. info.translated)
  end
  return table.concat(parts, ' ')
end

local function has_group_prefix(info, prefix)
  if info.direct and vim.startswith(info.direct, prefix) then
    return true
  end
  if info.translated and vim.startswith(info.translated, prefix) then
    return true
  end
  for _, name in ipairs(info.stack) do
    if vim.startswith(name, prefix) then
      return true
    end
  end
  return false
end

local function expected_string_group(corpus_case)
  if corpus_case.expectedStringKind == 'double' then
    return 'meshStringDouble'
  end
  if corpus_case.expectedStringKind == 'triple' then
    return 'meshStringTriple'
  end
  return nil
end

local function expected_markers(corpus_case)
  local markers = {}
  if corpus_case.expectedForms then
    for _, form in ipairs(corpus_case.expectedForms) do
      if form == 'dollar' then
        table.insert(markers, '${')
      elseif form == 'hash' then
        table.insert(markers, '#{')
      else
        fail('syntax', string.format('case=%s unknown_expected_form=%s', corpus_case.id, tostring(form)))
      end
    end
  end
  return markers
end

local function find_all_markers(lines, start_line, end_line, markers)
  local hits = {}
  for line_nr = start_line, end_line do
    local text = lines[line_nr]
    if text then
      for _, marker in ipairs(markers) do
        local init = 1
        while true do
          local col = string.find(text, marker, init, true)
          if not col then
            break
          end
          table.insert(hits, { line = line_nr, col = col, marker = marker })
          init = col + 1
        end
      end
    end
  end
  table.sort(hits, function(left, right)
    if left.line == right.line then
      return left.col < right.col
    end
    return left.line < right.line
  end)
  return hits
end

local function find_plain_string_column(lines, start_line, end_line)
  for line_nr = start_line, end_line do
    local text = lines[line_nr]
    if text then
      local first_quote = string.find(text, '"', 1, true)
      local start_col = first_quote and (first_quote + 1) or 1
      for col = start_col, #text do
        local char = text:sub(col, col)
        if char:match('[%a_]') then
          return line_nr, col
        end
      end
    end
  end
  return nil, nil
end

local function find_literal_position(lines, literal)
  for line_nr, text in ipairs(lines) do
    local col = string.find(text, literal, 1, true)
    if col then
      return line_nr, col
    end
  end
  return nil, nil
end

local function find_literal_position_or_fail(case_id, file_path, lines, literal, label)
  local line_nr, col = find_literal_position(lines, literal)
  if not line_nr or not col then
    fail('syntax', string.format('case=%s file=%s reason=missing_probe_literal label=%s literal=%s', case_id, rel(file_path), label, literal))
  end
  return line_nr, col
end

local function assert_mesh_syntax_fixture(case_id, file_path)
  if vim.fn.filereadable(file_path) ~= 1 then
    fail('syntax', string.format('case=%s file=%s reason=missing_fixture', case_id, rel(file_path)))
  end

  open_buffer(file_path)

  local filetype = vim.bo.filetype
  local current_syntax = vim.b.current_syntax or ''
  io.stdout:write(string.format(
    '[m036-s02] phase=syntax case=%s fixture=%s filetype=%s syntax=%s\n',
    case_id,
    rel(file_path),
    filetype,
    current_syntax
  ))

  if filetype ~= 'mesh' then
    fail('syntax', string.format('case=%s file=%s reason=wrong_filetype expected=mesh actual=%s', case_id, rel(file_path), filetype))
  end
  if current_syntax ~= 'mesh' then
    fail('syntax', string.format('case=%s file=%s reason=missing_mesh_syntax actual=%s', case_id, rel(file_path), current_syntax))
  end

  return vim.api.nvim_buf_get_lines(0, 0, -1, false)
end

local function assert_named_syntax_probe(case_id, file_path, probe, line_nr, col, opts)
  local info = syntax_info(line_nr, col)
  local detail = names_text(info)

  io.stdout:write(string.format(
    '[m036-s02] phase=syntax case=%s file=%s line=%d col=%d probe=%s %s\n',
    case_id,
    rel(file_path),
    line_nr,
    col,
    probe,
    detail
  ))

  if detail == '' and not opts.allow_empty then
    fail('syntax', string.format('case=%s file=%s line=%d col=%d probe=%s reason=no_syntax_group', case_id, rel(file_path), line_nr, col, probe))
  end

  if opts.expected_prefix and not has_group_prefix(info, opts.expected_prefix) then
    fail('syntax', string.format(
      'case=%s file=%s line=%d col=%d probe=%s reason=missing_%s_group %s',
      case_id,
      rel(file_path),
      line_nr,
      col,
      probe,
      opts.expected_label or opts.expected_prefix,
      detail
    ))
  end

  if opts.unexpected_prefix and has_group_prefix(info, opts.unexpected_prefix) then
    fail('syntax', string.format(
      'case=%s file=%s line=%d col=%d probe=%s reason=unexpected_%s_group %s',
      case_id,
      rel(file_path),
      line_nr,
      col,
      probe,
      opts.unexpected_label or opts.unexpected_prefix,
      detail
    ))
  end
end

local function run_unicode_identifier_boundary_syntax_case()
  local case_id = 'unicode-identifier-boundaries'
  local source_label = '<inline-unicode-identifiers>'
  local lines = {
    '〇name = 1',
    'ↀname = 2',
    '9abc = 3',
    'letⅣ = 4',
    'IntⅣ = 5',
    'SomeⅣ = 6',
    'trueⅣ = 7',
    'Ⅳalue = 8',
  }

  vim.cmd('silent! %bwipeout!')
  vim.cmd('enew')
  vim.api.nvim_buf_set_lines(0, 0, -1, false, lines)
  vim.cmd('setfiletype mesh')
  vim.cmd('syntax sync fromstart')

  if vim.b.current_syntax ~= 'mesh' then
    fail('syntax', string.format('case=%s file=%s reason=missing_mesh_syntax actual=%s', case_id, source_label, vim.b.current_syntax or ''))
  end

  local probes = {
    { 'unicode-letter-number-start', 1, 1, 'meshVariable', nil },
    { 'unicode-letter-number-tail', 1, #'〇name', 'meshVariable', nil },
    { 'unicode-roman-numeral-start', 2, 1, 'meshVariable', nil },
    { 'unicode-roman-numeral-tail', 2, #'ↀname', 'meshVariable', nil },
    { 'ascii-numeric-start-suffix', 3, 2, nil, 'meshVariable', true },
    { 'keyword-prefix-start', 4, 1, 'meshVariable', 'meshDeclarationKeyword' },
    { 'keyword-prefix-tail', 4, #'let' + 1, 'meshVariable', 'meshDeclarationKeyword' },
    { 'builtin-type-prefix-start', 5, 1, 'meshType', 'meshBuiltinType' },
    { 'builtin-type-prefix-tail', 5, #'Int' + 1, 'meshType', 'meshBuiltinType' },
    { 'builtin-constructor-prefix-start', 6, 1, 'meshType', 'meshBuiltinConstructor' },
    { 'builtin-constructor-prefix-tail', 6, #'Some' + 1, 'meshType', 'meshBuiltinConstructor' },
    { 'boolean-prefix-start', 7, 1, 'meshVariable', 'meshBoolean' },
    { 'boolean-prefix-tail', 7, #'true' + 1, 'meshVariable', 'meshBoolean' },
    { 'unicode-non-lu-uppercase-start', 8, 1, 'meshVariable', 'meshType' },
    { 'unicode-non-lu-uppercase-tail', 8, #'Ⅳalue', 'meshVariable', 'meshType' },
  }

  for _, probe in ipairs(probes) do
    assert_named_syntax_probe(case_id, source_label, probe[1], probe[2], probe[3], {
      expected_prefix = probe[4],
      expected_label = probe[4],
      unexpected_prefix = probe[5],
      unexpected_label = probe[5],
      allow_empty = probe[6],
    })
  end

  return #probes
end

local function run_cluster_decorator_syntax_case()
  local case_id = 'cluster-decorators'
  local fixture_path = vim.fs.joinpath(repo_root, 'scripts', 'fixtures', 'cluster-decorators.mpl')
  local lines = assert_mesh_syntax_fixture(case_id, fixture_path)

  local plain_line, plain_col = find_literal_position_or_fail(case_id, fixture_path, lines, '@cluster pub fn add()', 'plain-decorator')
  local counted_line, counted_col = find_literal_position_or_fail(case_id, fixture_path, lines, '@cluster(3) pub fn sync_todos()', 'counted-decorator')
  local bare_line, bare_col = find_literal_position_or_fail(case_id, fixture_path, lines, 'let cluster = 1', 'bare-identifier')

  assert_named_syntax_probe(case_id, fixture_path, 'plain-decorator-sigil', plain_line, plain_col, {
    expected_prefix = 'meshCluster',
    expected_label = 'cluster',
    unexpected_prefix = 'meshVariable',
    unexpected_label = 'variable',
  })
  assert_named_syntax_probe(case_id, fixture_path, 'plain-decorator-name', plain_line, plain_col + 1, {
    expected_prefix = 'meshCluster',
    expected_label = 'cluster',
    unexpected_prefix = 'meshVariable',
    unexpected_label = 'variable',
  })
  assert_named_syntax_probe(case_id, fixture_path, 'counted-decorator-sigil', counted_line, counted_col, {
    expected_prefix = 'meshCluster',
    expected_label = 'cluster',
    unexpected_prefix = 'meshVariable',
    unexpected_label = 'variable',
  })
  assert_named_syntax_probe(case_id, fixture_path, 'counted-decorator-name', counted_line, counted_col + 1, {
    expected_prefix = 'meshCluster',
    expected_label = 'cluster',
    unexpected_prefix = 'meshVariable',
    unexpected_label = 'variable',
  })
  assert_named_syntax_probe(case_id, fixture_path, 'counted-decorator-count', counted_line, counted_col + #'@cluster(', {
    expected_prefix = 'meshNumberInteger',
    expected_label = 'number',
    unexpected_prefix = 'meshVariable',
    unexpected_label = 'variable',
  })
  assert_named_syntax_probe(case_id, fixture_path, 'bare-cluster-identifier', bare_line, bare_col + #'let ', {
    expected_prefix = 'meshVariable',
    expected_label = 'variable',
    unexpected_prefix = 'meshCluster',
    unexpected_label = 'cluster',
  })

  return 6
end

local function run_current_syntax_surface_case()
  local case_id = 'current-syntax-surface'
  local fixture_path = vim.fs.joinpath(repo_root, 'scripts', 'fixtures', 'current-syntax-surface.mpl')
  local lines = assert_mesh_syntax_fixture(case_id, fixture_path)
  local probes = {
    { 'module-doc-comment', '##! Current syntax surface fixture.', 0, 'meshDocModuleComment' },
    { 'doc-comment', '## Compiler-derived highlighting probes.', 0, 'meshDocComment' },
    { 'line-comment', '# This fixture intentionally ends', 0, 'meshComment' },
    { 'nested-block-comment', '#= outer block #= nested block =# still outer block =#', #'#= outer block #= nested block =# ', 'meshCommentBlock' },
    { 'native-decorator', '@native("mesh_u128_add")', 1, 'meshNativeDecorator', 'meshVariable' },
    { 'spaced-native-decorator', '@ native ("mesh_u128_identity")', 2, 'meshNativeDecorator', 'meshVariable' },
    { 'spaced-cluster-decorator', '@ cluster ( 3 )', 2, 'meshClusterDecorator', 'meshVariable' },
    { 'spaced-cluster-count', '@ cluster ( 3 )', #'@ cluster ( ', 'meshNumberInteger' },
    { 'function-declaration', 'pub fn native_add(', #'pub fn ', 'meshFunctionName', 'meshVariable' },
    { 'builtin-type', 'left :: U128', #'left :: ', 'meshBuiltinType' },
    { 'from-import', 'from Solana.Read import pubkey', 0, 'meshImportKeyword', 'meshVariable' },
    { 'module-path', 'from Solana.Read import pubkey', #'from ', 'meshModulePath' },
    { 'unicode-type-declaration', 'pub type Δelta = Int', #'pub type ', 'meshTypeDeclarationName', 'meshVariable' },
    { 'struct-declaration-keyword', 'pub struct User do', #'pub ', 'meshDeclarationKeyword', 'meshTypeDeclarationName' },
    { 'parameterless-interface-signature', 'fn ping -> Int', #'fn ', 'meshFunctionName', 'meshVariable' },
    { 'default-interface-method', 'fn default_ping -> Int do', #'fn ', 'meshFunctionName', 'meshVariable' },
    { 'interface-method-after-multiline-closure', 'fn after_multiline_default', #'fn ', 'meshFunctionName', 'meshVariable' },
    { 'bare-interface-signature', 'fn reset', #'fn ', 'meshFunctionName', 'meshVariable' },
    { 'semicolon-interface-signature', 'fn semicolon_reset', #'fn ', 'meshFunctionName', 'meshVariable' },
    { 'comment-trivia-interface-name', 'CommentTriviaHealthProbe #= interface-do trivia =# do', 0, 'meshTypeDeclarationName', 'meshVariable' },
    { 'comment-trivia-interface-method', 'commented_ping', 0, 'meshFunctionName', 'meshVariable' },
    { 'orm-table', 'table "users"', 0, 'meshOrmKeyword', 'meshVariable' },
    { 'orm-primary-key', 'primary_key :uuid', 0, 'meshOrmKeyword', 'meshVariable' },
    { 'orm-timestamps', 'timestamps true', 0, 'meshOrmKeyword', 'meshVariable' },
    { 'orm-timestamps-false', 'timestamps false', 0, 'meshOrmKeyword', 'meshVariable' },
    { 'orm-belongs-to', 'belongs_to :account', 0, 'meshOrmKeyword', 'meshVariable' },
    { 'orm-has-one', 'has_one :profile', 0, 'meshOrmKeyword', 'meshVariable' },
    { 'orm-has-many', 'has_many :posts', 0, 'meshOrmKeyword', 'meshVariable' },
    { 'orm-deriving', 'end deriving(Schema', #'end ', 'meshOrmKeyword', 'meshVariable' },
    { 'supervisor-strategy', 'strategy: one_for_all', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'supervisor-value', 'strategy: one_for_all', #'strategy: ', 'meshSupervisorValue', 'meshVariable' },
    { 'supervisor-one-for-one', 'strategy: one_for_one', #'strategy: ', 'meshSupervisorValue', 'meshVariable' },
    { 'supervisor-rest-for-one', 'strategy: rest_for_one', #'strategy: ', 'meshSupervisorValue', 'meshVariable' },
    { 'supervisor-simple-one-for-one', 'strategy: simple_one_for_one', #'strategy: ', 'meshSupervisorValue', 'meshVariable' },
    { 'supervisor-max-restarts', 'max_restarts: 5', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'supervisor-max-seconds', 'max_seconds: 10', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'supervisor-child', 'child WorkerPool do', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'supervisor-start', 'start: fn ->', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'supervisor-restart', 'restart: permanent', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'supervisor-transient', 'restart: transient', #'restart: ', 'meshSupervisorValue', 'meshVariable' },
    { 'supervisor-temporary', 'restart: temporary', #'restart: ', 'meshSupervisorValue', 'meshVariable' },
    { 'supervisor-shutdown', 'shutdown: brutal_kill', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'supervisor-numeric-shutdown', 'shutdown: 5000', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'split-do-restart-after-closure', 'restart: permanent # split-do-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'wrapped-start-restart-after-closure', 'restart: permanent # wrapped-start-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'multiline-wrapped-start-restart-after-closure', 'restart: temporary # multiline-wrapped-start-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'wrapped-do-restart-after-closure', 'restart: transient # wrapped-do-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'marker-aware-arrow-restart-after-closure', 'restart: permanent # marker-aware-arrow-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'marker-aware-do-restart-after-closure', 'restart: transient # marker-aware-do-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'multiline-header-restart-after-closure', 'restart: temporary # multiline-header-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'lexically-masked-header-restart-after-closure', 'restart: permanent # lexically-masked-header-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'trailing-do-closure-restart-after-closure', 'restart: transient # trailing-do-closure-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'case-arm-do-closure-restart-after-closure', 'restart: temporary # case-arm-do-closure-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'empty-closure-restart-after-closure', 'restart: permanent # empty-closure-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'body-lexical-end-restart-after-closure', 'restart: transient # body-lexical-end-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'else-if-restart-after-closure', 'restart: temporary # else-if-closure-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'multiline-comment-else-if-restart-after-closure', 'restart: permanent # multiline-comment-else-if-closure-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'nested-guard-closure-restart-after-closure', 'restart: permanent # nested-guard-closure-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'typed-closure-restart-after-closure', 'restart: transient # typed-closure-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'guard-block-closure-restart-after-closure', 'restart: temporary # guard-block-closure-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'nested-declaration-restart-after-closure', 'restart: permanent # nested-declaration-restart', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'semicolon-supervisor-clause', 'max_restarts: 13', 0, 'meshSupervisorKeyword', 'meshVariable' },
    { 'semicolon-supervisor-limit', 'strategy: one_for_one; max_restarts: 3;', #'strategy: one_for_one; ', 'meshSupervisorKeyword', 'meshVariable' },
    { 'semicolon-supervisor-child', '; child InlineWorker do', #'; ', 'meshSupervisorKeyword', 'meshVariable' },
    { 'semicolon-child-restart', 'end; restart: transient;', #'end; ', 'meshSupervisorKeyword', 'meshVariable' },
    { 'semicolon-child-restart-value', 'end; restart: transient;', #'end; restart: ', 'meshSupervisorValue', 'meshVariable' },
    { 'semicolon-child-shutdown', '; shutdown: brutal_kill', #'; ', 'meshSupervisorKeyword', 'meshVariable' },
    { 'post-child-semicolon-supervisor-limit', 'end; max_seconds: 7', #'end; ', 'meshSupervisorKeyword', 'meshVariable' },
    { 'nested-start-ordinary-strategy-key', '        strategy: one_for_one,', 8, 'meshVariable', 'meshSupervisorKeyword' },
    { 'nested-start-ordinary-strategy-value', '        strategy: one_for_one,', #'        strategy: ', 'meshVariable', 'meshSupervisorValue' },
    { 'nested-start-ordinary-restart-key', '        restart: transient,', 8, 'meshVariable', 'meshSupervisorKeyword' },
    { 'nested-start-ordinary-shutdown-key', '        shutdown: brutal_kill,', 8, 'meshVariable', 'meshSupervisorKeyword' },
    { 'parameterless-call-handler', 'call Get :: Int', #'call ', 'meshHandlerName', 'meshType' },
    { 'parameterless-cast-handler', 'cast Reset do', #'cast ', 'meshHandlerName', 'meshType' },
    { 'parameterless-function-declaration', 'pub fn heartbeat -> Bool', #'pub fn ', 'meshFunctionName', 'meshVariable' },
    { 'parameterless-def-declaration', 'pub def heartbeat_alias -> Bool', #'pub def ', 'meshFunctionName', 'meshVariable' },
    { 'ambiguous-private-return-typed-function-name', 'fn private_heartbeat -> Bool', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'private-parameterless-def-declaration', 'def private_heartbeat_alias -> Bool', #'def ', 'meshFunctionName', 'meshVariable' },
    { 'ambiguous-private-tuple-result-function-name', 'fn private_pair -> (Int, String)', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'ambiguous-guarded-expression-function-name', 'fn guarded_tick when ready = ready', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'ambiguous-typed-expression-function-name', 'fn typed_expression -> Int = 42', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'ambiguous-guarded-block-declaration-name', 'fn guarded_block when ready do', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'result-type-bang', 'User!String', #'User', 'meshResultOperator' },
    { 'module-call', 'Repo.one(scoped)', 0, 'meshModuleCall' },
    { 'module-call-function', 'Repo.one(scoped)', #'Repo.', 'meshModuleFunctionCall' },
    { 'map-opener', 'Changeset.cast(%{},', #'Changeset.cast(', 'meshMapDelimiter' },
    { 'struct-update-bar', '%{user | name:', #'%{user ', 'meshBarOperator' },
    { 'escaped-regex', '~r/users\\/[0-9]+/ims', 0, 'meshRegex' },
    { 'multiline-regex-opener', '~r/^first$', 0, 'meshRegex' },
    { 'multiline-regex-continuation', '  ^second$/ms', 2, 'meshRegex' },
    { 'multiline-regex-flags', '  ^second$/ms', #'  ^second$/', 'meshRegex' },
    { 'after-multiline-regex', 'let after_multiline_matcher = 1', #'let ', 'meshVariable', 'meshRegex' },
    { 'ordering-less-constructor', '[Less, Equal, Greater]', 1, 'meshBuiltinConstructor', 'meshType' },
    { 'ordering-equal-constructor', '[Less, Equal, Greater]', #'[Less, ', 'meshBuiltinConstructor', 'meshType' },
    { 'ordering-greater-constructor', '[Less, Equal, Greater]', #'[Less, Equal, ', 'meshBuiltinConstructor', 'meshType' },
    { 'some-constructor', '[Some(1), None, Ok(1), Err("error")]', 1, 'meshBuiltinConstructor', 'meshType' },
    { 'none-constructor', '[Some(1), None, Ok(1), Err("error")]', #'[Some(1), ', 'meshBuiltinConstructor', 'meshType' },
    { 'ok-constructor', '[Some(1), None, Ok(1), Err("error")]', #'[Some(1), None, ', 'meshBuiltinConstructor', 'meshType' },
    { 'err-constructor', '[Some(1), None, Ok(1), Err("error")]', #'[Some(1), None, Ok(1), ', 'meshBuiltinConstructor', 'meshType' },
    { 'list-iterator-type', '[ListIterator, MapIterator, SetIterator, RangeIterator]', 1, 'meshBuiltinType', 'meshType' },
    { 'map-iterator-type', '[ListIterator, MapIterator, SetIterator, RangeIterator]', #'[ListIterator, ', 'meshBuiltinType', 'meshType' },
    { 'set-iterator-type', '[ListIterator, MapIterator, SetIterator, RangeIterator]', #'[ListIterator, MapIterator, ', 'meshBuiltinType', 'meshType' },
    { 'range-iterator-type', '[ListIterator, MapIterator, SetIterator, RangeIterator]', #'[ListIterator, MapIterator, SetIterator, ', 'meshBuiltinType', 'meshType' },
    { 'decimal-integer', 'numeric_literals = [42,', #'numeric_literals = [', 'meshNumberInteger' },
    { 'hex-integer', '0xFF', 0, 'meshNumberHex' },
    { 'binary-integer', '0b1010', 0, 'meshNumberBinary' },
    { 'octal-integer', '0o77', 0, 'meshNumberOctal' },
    { 'decimal-float', '3.14', 0, 'meshNumberFloat' },
    { 'exponent-float', '1.0e10', 0, 'meshNumberFloat' },
    { 'closure-parameter', 'fn value -> value end', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'guarded-closure-parameter', 'fn value when value > 0 -> value end', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'line-leading-guarded-closure-parameter', '    fn value when value > 0 -> value end,', #'    fn ', 'meshVariable', 'meshFunctionName' },
    { 'multiline-uppercase-closure-parameter', 'fn closure_value -> Int', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'equality-guarded-closure-parameter', 'fn equal_value when equal_value == 0 -> equal_value end', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'guarded-do-closure-parameter', 'fn guarded_do_value when guarded_do_value > 0 do', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'constructor-pattern-closure', 'fn Some(constructor_value) -> constructor_value end', #'fn ', 'meshBuiltinConstructor', 'meshFunctionName' },
    { 'string-equals-closure-parameter', 'fn option_value -> Some("a=b") end', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'trailing-do-closure-parameter', 'fn builder_value -> Builder.make() do', #'fn ', 'meshVariable', 'meshFunctionName' },
    { 'private-parameterized-function-declaration', 'fn contextual_names(native, table', #'fn ', 'meshFunctionName', 'meshVariable' },
    { 'comment-trivia-function-declaration', 'fn #= declaration-name trivia =# helper = 1', #'fn #= declaration-name trivia =# ', 'meshFunctionName', 'meshVariable' },
    { 'ordinary-strategy-key', 'configure(strategy: one_for_one', #'configure(', 'meshVariable', 'meshSupervisorKeyword' },
    { 'ordinary-strategy-value', 'configure(strategy: one_for_one', #'configure(strategy: ', 'meshVariable', 'meshSupervisorValue' },
    { 'ordinary-restart-key', 'restart: transient, shutdown:', 0, 'meshVariable', 'meshSupervisorKeyword' },
    { 'ordinary-restart-value', 'restart: transient, shutdown:', #'restart: ', 'meshVariable', 'meshSupervisorValue' },
    { 'ordinary-shutdown-key', 'shutdown: brutal_kill)', 0, 'meshVariable', 'meshSupervisorKeyword' },
    { 'ordinary-shutdown-value', 'shutdown: brutal_kill)', #'shutdown: ', 'meshVariable', 'meshSupervisorValue' },
    { 'multiline-ordinary-strategy-key', '    strategy: one_for_one,', 4, 'meshVariable', 'meshSupervisorKeyword' },
    { 'multiline-ordinary-strategy-value', '    strategy: one_for_one,', #'    strategy: ', 'meshVariable', 'meshSupervisorValue' },
    { 'multiline-ordinary-restart-key', '    restart: transient,', 4, 'meshVariable', 'meshSupervisorKeyword' },
    { 'multiline-ordinary-restart-value', '    restart: transient,', #'    restart: ', 'meshVariable', 'meshSupervisorValue' },
    { 'multiline-ordinary-shutdown-key', '    shutdown: brutal_kill,', 4, 'meshVariable', 'meshSupervisorKeyword' },
    { 'multiline-ordinary-shutdown-value', '    shutdown: brutal_kill,', #'    shutdown: ', 'meshVariable', 'meshSupervisorValue' },
    { 'ordinary-deriving-call', 'deriving(value)', 0, 'meshFunctionCall', 'meshOrmKeyword' },
    { 'or-pattern-bar', 'Ok(value) | Some(value)', #'Ok(value) ', 'meshBarOperator' },
    { 'as-pattern', 'None as missing', #'None ', 'meshContextualKeyword', 'meshVariable' },
    { 'wildcard', '_ -> updated', 0, 'meshWildcard', 'meshVariable' },
    { 'unicode-identifier', 'let κόσμος = pubkey()', #'let ', 'meshVariable' },
    { 'unicode-letter-number-identifier', 'let Ⅳalue = 4', #'let ', 'meshVariable' },
    { 'unicode-uncased-identifier', 'let 四季 = 4', #'let ', 'meshVariable' },
    { 'bare-function-call', 'println("#{κόσμος}")', 0, 'meshFunctionCall', 'meshVariable' },
    { 'interpolated-identifier', 'println("#{κόσμος}")', #'println("#{', 'meshVariable', 'meshTypeDeclarationName' },
    { 'unary-bang', '!false', 0, 'meshResultOperator' },
    { 'plain-native', 'fn contextual_names(native,', #'fn contextual_names(', 'meshVariable', 'meshNativeDecorator' },
    { 'plain-table', 'native, table, strategy', #'native, ', 'meshVariable', 'meshOrmKeyword' },
    { 'plain-strategy', 'table, strategy, deriving', #'table, ', 'meshVariable', 'meshSupervisorKeyword' },
    { 'plain-deriving', 'strategy, deriving, from', #'strategy, ', 'meshVariable', 'meshOrmKeyword' },
    { 'plain-from', 'deriving, from, as', #'deriving, ', 'meshVariable', 'meshImportKeyword' },
    { 'plain-as', 'from, as) do', #'from, ', 'meshVariable', 'meshContextualKeyword' },
    { 'unicode-keyword-prefix-identifier', 'unicode_boundary_names(letπ,', #'unicode_boundary_names(', 'meshVariable', 'meshDeclarationKeyword' },
    { 'unicode-builtin-prefix-identifier', 'letπ, Inté, Someδ, trueλ', #'letπ, ', 'meshType', 'meshBuiltinType' },
    { 'unicode-constructor-prefix-identifier', 'letπ, Inté, Someδ, trueλ', #'letπ, Inté, ', 'meshType', 'meshBuiltinConstructor' },
    { 'unicode-constant-prefix-identifier', 'letπ, Inté, Someδ, trueλ', #'letπ, Inté, Someδ, ', 'meshVariable', 'meshBoolean' },
    { 'unicode-cluster-prefix-identifier', '@clusterπ', 1, 'meshVariable', 'meshClusterDecorator' },
    { 'unicode-native-prefix-identifier', '@nativeπ("symbol")', 1, 'meshFunctionCall', 'meshNativeDecorator' },
    { 'invalid-uppercase-atom', ':Invalid', 0, 'meshPunctuation', 'meshAtom' },
    { 'invalid-slot-pipe-zero', 'left |0> right', #'left ', 'meshInvalidSlotPipe', 'meshPipeOperator' },
    { 'invalid-slot-pipe-one', 'left |1> right', #'left ', 'meshInvalidSlotPipe', 'meshPipeOperator' },
    { 'incomplete-slot-pipe', 'left |2 right', #'left ', 'meshInvalidSlotPipe', 'meshBarOperator' },
  }

  for _, probe in ipairs(probes) do
    local line_nr, col = find_literal_position_or_fail(case_id, fixture_path, lines, probe[2], probe[1])
    assert_named_syntax_probe(case_id, fixture_path, probe[1], line_nr, col + probe[3], {
      expected_prefix = probe[4],
      expected_label = probe[4],
      unexpected_prefix = probe[5],
      unexpected_label = probe[5],
    })
  end

  local incomplete_regex_line
  for line_nr, text in ipairs(lines) do
    if text == '~r' then
      incomplete_regex_line = line_nr
      break
    end
  end
  if not incomplete_regex_line then
    fail('syntax', string.format('case=%s file=%s reason=missing_probe_literal label=incomplete-regex literal=~r', case_id, rel(fixture_path)))
  end
  assert_named_syntax_probe(case_id, fixture_path, 'incomplete-regex', incomplete_regex_line, 2, {
    expected_prefix = 'meshVariable',
    expected_label = 'meshVariable',
    unexpected_prefix = 'meshRegex',
    unexpected_label = 'meshRegex',
  })

  return #probes + 1
end

local function run_compiler_token_syntax_case()
  local case_id = 'compiler-token-surface'
  local token_source_path = vim.fs.joinpath(repo_root, 'compiler', 'mesh-common', 'src', 'token.rs')
  local token_source = read_file(token_source_path)
  local function token_variants(section_name, next_section_name)
    local section = token_source:match('// ── ' .. section_name .. '.-// ── ' .. next_section_name)
    if not section then
      fail('syntax', string.format('case=%s reason=missing_token_section section=%s', case_id, section_name))
    end
    local variants = {}
    for line in section:gmatch('[^\n]+') do
      local variant = line:match('^    ([A-Z][A-Za-z0-9_]*)[%s%(,]')
      if variant then
        variants[variant] = true
      end
    end
    return variants
  end
  local function assert_variant_set(section_name, next_section_name, expected)
    local actual = token_variants(section_name, next_section_name)
    local expected_set = {}
    for _, variant in ipairs(expected) do
      expected_set[variant] = true
    end
    if vim.tbl_count(actual) ~= vim.tbl_count(expected_set) then
      fail('syntax', string.format(
        'case=%s reason=token_probe_count_drift section=%s compiler=%d probes=%d',
        case_id,
        section_name,
        vim.tbl_count(actual),
        vim.tbl_count(expected_set)
      ))
    end
    for variant in pairs(actual) do
      if not expected_set[variant] then
        fail('syntax', string.format('case=%s reason=compiler_token_missing_probe section=%s variant=%s', case_id, section_name, variant))
      end
    end
    for variant in pairs(expected_set) do
      if not actual[variant] then
        fail('syntax', string.format('case=%s reason=stale_token_probe section=%s variant=%s', case_id, section_name, variant))
      end
    end
  end
  local keyword_groups = {
    meshControlKeyword = { 'if', 'else', 'case', 'match', 'when', 'return', 'import', 'for', 'while', 'cond', 'break', 'continue' },
    meshDoKeyword = { 'do' },
    meshEndKeyword = { 'end' },
    meshDeclarationKeyword = { 'fn', 'let', 'def', 'type', 'struct', 'module', 'interface', 'impl', 'pub', 'actor', 'service', 'supervisor', 'call', 'cast', 'trait', 'alias', 'json' },
    meshWordOperator = { 'and', 'or', 'not', 'in', 'where', 'with', 'spawn', 'send', 'receive', 'self', 'link', 'monitor', 'terminate', 'trap', 'after' },
    meshBoolean = { 'true', 'false', 'nil' },
  }
  local operator_probes = {
    { 'Plus', '+', 'meshArithmeticOperator' },
    { 'Minus', '-', 'meshArithmeticOperator' },
    { 'Star', '*', 'meshArithmeticOperator' },
    { 'Slash', '/', 'meshArithmeticOperator' },
    { 'Percent', '%', 'meshArithmeticOperator' },
    { 'EqEq', '==', 'meshComparisonOperator' },
    { 'NotEq', '!=', 'meshComparisonOperator' },
    { 'Lt', '<', 'meshComparisonOperator' },
    { 'Gt', '>', 'meshComparisonOperator' },
    { 'LtEq', '<=', 'meshComparisonOperator' },
    { 'GtEq', '>=', 'meshComparisonOperator' },
    { 'AmpAmp', '&&', 'meshLogicalAndOperator' },
    { 'PipePipe', '||', 'meshLogicalOrOperator' },
    { 'Bang', '!', 'meshResultOperator' },
    { 'Pipe', '|>', 'meshPipeOperator' },
    { 'SlotPipe', '|2>', 'meshSlotPipeOperator' },
    { 'DotDot', '..', 'meshRangeOperator' },
    { 'Diamond', '<>', 'meshDiamondOperator' },
    { 'PlusPlus', '++', 'meshConcatOperator' },
    { 'Eq', '=', 'meshAssignmentOperator' },
    { 'Arrow', '->', 'meshArrowOperator' },
    { 'FatArrow', '=>', 'meshFatArrowOperator' },
    { 'ColonColon', '::', 'meshAnnotationOperator' },
    { 'Question', '?', 'meshTryOperator' },
    { 'Bar', '|', 'meshBarOperator' },
  }
  local delimiter_probes = {
    { 'LParen', '(', 'meshPunctuation' },
    { 'RParen', ')', 'meshPunctuation' },
    { 'LBracket', '[', 'meshPunctuation' },
    { 'RBracket', ']', 'meshPunctuation' },
    { 'LBrace', '{', 'meshPunctuation' },
    { 'RBrace', '}', 'meshPunctuation' },
  }
  local punctuation_probes = {
    { 'Comma', ',', 'meshPunctuation' },
    { 'Dot', '.', 'meshPunctuation' },
    { 'Colon', ':', 'meshPunctuation' },
    { 'Semicolon', ';', 'meshPunctuation' },
    { 'At', '@', 'meshPunctuation' },
  }
  local builtin_types = {
    'Atom', 'Bool', 'BootstrapStatus', 'Bytes', 'ContinuityAuthorityStatus',
    'ContinuityRecord', 'ContinuitySubmitDecision', 'DateTime', 'Float', 'Fun',
    'HttpClientMetrics', 'HttpResponse', 'I128', 'Int', 'Json', 'List',
    'ListIterator', 'Map', 'MapIterator', 'Never', 'Option', 'Ordering', 'PgConn',
    'Pid', 'PoolHandle', 'Queue', 'Range', 'RangeIterator', 'Regex', 'Request',
    'Response', 'Result', 'Router', 'Set', 'SetIterator', 'SqliteConn', 'String',
    'Tuple', 'U128', 'U64', 'Unit', 'WsMessage',
  }

  local compiler_keywords = {}
  for keyword in token_source:gmatch('"([a-z_]+)"%s*=>%s*Some%(TokenKind::') do
    compiler_keywords[keyword] = true
  end
  local expected_keywords = {}
  local keyword_count = 0
  for _, keywords in pairs(keyword_groups) do
    for _, keyword in ipairs(keywords) do
      expected_keywords[keyword] = true
      keyword_count = keyword_count + 1
    end
  end
  for keyword in pairs(compiler_keywords) do
    if not expected_keywords[keyword] then
      fail('syntax', string.format('case=%s reason=compiler_keyword_missing_probe keyword=%s', case_id, keyword))
    end
  end
  for keyword in pairs(expected_keywords) do
    if not compiler_keywords[keyword] then
      fail('syntax', string.format('case=%s reason=stale_keyword_probe keyword=%s', case_id, keyword))
    end
  end
  if keyword_count ~= 49 then
    fail('syntax', string.format('case=%s reason=unexpected_keyword_count expected=49 actual=%d', case_id, keyword_count))
  end

  assert_variant_set('Operators', 'Delimiters', vim.tbl_map(function(probe) return probe[1] end, operator_probes))
  assert_variant_set('Delimiters', 'Punctuation', vim.tbl_map(function(probe) return probe[1] end, delimiter_probes))
  local punctuation_variants = vim.tbl_map(function(probe) return probe[1] end, punctuation_probes)
  table.insert(punctuation_variants, 'Newline')
  assert_variant_set('Punctuation', 'Literals', punctuation_variants)
  assert_variant_set('Literals', 'Identifiers and comments', {
    'IntLiteral', 'FloatLiteral', 'StringStart', 'StringEnd', 'StringContent',
    'InterpolationStart', 'InterpolationEnd', 'Atom', 'RegexLiteral',
  })
  assert_variant_set('Identifiers and comments', 'Special', {
    'Ident', 'Comment', 'DocComment', 'ModuleDocComment',
  })

  local fixture_path = vim.fs.joinpath(repo_root, '.tmp', 'm036-s02', requested_phase, 'compiler-token-surface.mpl')
  local fixture_lines = {}
  local assertions = {}
  for group, keywords in pairs(keyword_groups) do
    for _, keyword in ipairs(keywords) do
      table.insert(fixture_lines, keyword)
      table.insert(assertions, { #fixture_lines, 1, 'keyword-' .. keyword, group })
    end
  end
  for _, probe in ipairs(operator_probes) do
    table.insert(fixture_lines, 'left ' .. probe[2] .. ' right')
    table.insert(assertions, { #fixture_lines, 6, 'operator-' .. probe[1], probe[3] })
  end
  for _, probe in ipairs(delimiter_probes) do
    table.insert(fixture_lines, probe[2])
    table.insert(assertions, { #fixture_lines, 1, 'delimiter-' .. probe[1], probe[3] })
  end
  for _, probe in ipairs(punctuation_probes) do
    table.insert(fixture_lines, probe[2])
    table.insert(assertions, { #fixture_lines, 1, 'punctuation-' .. probe[1], probe[3] })
  end
  for _, builtin_type in ipairs(builtin_types) do
    table.insert(fixture_lines, builtin_type)
    table.insert(assertions, { #fixture_lines, 1, 'builtin-type-' .. builtin_type, 'meshBuiltinType' })
  end
  write_file(fixture_path, fixture_lines)
  assert_mesh_syntax_fixture(case_id, fixture_path)

  for _, assertion in ipairs(assertions) do
    assert_named_syntax_probe(case_id, fixture_path, assertion[3], assertion[1], assertion[2], {
      expected_prefix = assertion[4],
      expected_label = assertion[4],
    })
  end

  return #assertions
end

local function load_syntax_cases()
  local corpus_path = assert(os.getenv('MESH_NVIM_CASES_JSON'), 'MESH_NVIM_CASES_JSON is required for syntax smoke')
  local corpus = decode_json_file(corpus_path)
  local selected_cases = {}

  for _, corpus_case in ipairs(corpus.cases or {}) do
    local materialized_path = corpus_case.materializedPath
    if type(materialized_path) ~= 'string' or materialized_path == '' then
      fail('syntax', string.format('case=%s missing_materialized_path', tostring(corpus_case.id)))
    end
    if materialized_path:sub(-4) ~= '.mpl' then
      fail('syntax', string.format('case=%s materialized_path_is_not_mpl path=%s', tostring(corpus_case.id), materialized_path))
    end
    table.insert(selected_cases, corpus_case)
  end

  table.sort(selected_cases, function(left, right)
    if left.sourcePath == right.sourcePath then
      return left.sourceStartLine < right.sourceStartLine
    end
    return left.sourcePath < right.sourcePath
  end)

  if #selected_cases == 0 then
    fail('syntax', 'reason=no_materialized_cases_selected corpus=' .. corpus_path)
  end

  return selected_cases, corpus_path
end

local function run_syntax_phase()
  local selected_cases, corpus_path = load_syntax_cases()

  vim.cmd('filetype on')
  vim.cmd('syntax enable')

  io.stdout:write(string.format('[m036-s02] phase=syntax corpus=%s selected_cases=%d\n', rel(corpus_path), #selected_cases))

  for _, corpus_case in ipairs(selected_cases) do
    local abs_path = vim.fs.joinpath(repo_root, corpus_case.materializedPath)
    if vim.fn.filereadable(abs_path) ~= 1 then
      fail('syntax', string.format('case=%s materialized=%s reason=missing_materialized_fixture', corpus_case.id, corpus_case.materializedPath))
    end

    open_buffer(abs_path)

    local filetype = vim.bo.filetype
    local current_syntax = vim.b.current_syntax or ''
    io.stdout:write(string.format(
      '[m036-s02] phase=syntax case=%s source=%s:%d-%d materialized=%s filetype=%s syntax=%s\n',
      corpus_case.id,
      corpus_case.sourcePath,
      corpus_case.sourceStartLine,
      corpus_case.sourceEndLine,
      corpus_case.materializedPath,
      filetype,
      current_syntax
    ))

    if filetype ~= 'mesh' then
      fail('syntax', string.format('case=%s file=%s reason=wrong_filetype expected=mesh actual=%s', corpus_case.id, corpus_case.materializedPath, filetype))
    end
    if current_syntax ~= 'mesh' then
      fail('syntax', string.format('case=%s file=%s reason=missing_mesh_syntax actual=%s', corpus_case.id, corpus_case.materializedPath, current_syntax))
    end

    local lines = vim.api.nvim_buf_get_lines(0, 0, -1, false)
    local string_group = expected_string_group(corpus_case)
    local markers = expected_markers(corpus_case)
    local hits = find_all_markers(lines, corpus_case.startLine, corpus_case.endLine, markers)

    if #markers > 0 and #hits == 0 then
      fail('syntax', string.format('case=%s file=%s reason=missing_expected_interpolation range=%d-%d', corpus_case.id, corpus_case.materializedPath, corpus_case.startLine, corpus_case.endLine))
    end

    if corpus_case.expectNoInterpolation and #find_all_markers(lines, corpus_case.startLine, corpus_case.endLine, { '${', '#{' }) > 0 then
      fail('syntax', string.format('case=%s file=%s reason=unexpected_interpolation_marker range=%d-%d', corpus_case.id, corpus_case.materializedPath, corpus_case.startLine, corpus_case.endLine))
    end

    for _, hit in ipairs(hits) do
      local info = syntax_info(hit.line, hit.col)
      io.stdout:write(string.format(
        '[m036-s02] phase=syntax case=%s source=%s materialized=%s line=%d col=%d marker=%s %s\n',
        corpus_case.id,
        corpus_case.sourcePath,
        corpus_case.materializedPath,
        hit.line,
        hit.col,
        hit.marker,
        names_text(info)
      ))
      if names_text(info) == '' then
        fail('syntax', string.format('case=%s file=%s line=%d col=%d reason=no_syntax_group', corpus_case.id, corpus_case.materializedPath, hit.line, hit.col))
      end
      if not has_group_prefix(info, 'meshInterpolation') then
        fail('syntax', string.format('case=%s file=%s line=%d col=%d reason=missing_interpolation_group %s', corpus_case.id, corpus_case.materializedPath, hit.line, hit.col, names_text(info)))
      end
      if string_group and not has_group_prefix(info, string_group) then
        fail('syntax', string.format('case=%s file=%s line=%d col=%d reason=wrong_string_kind expected=%s %s', corpus_case.id, corpus_case.materializedPath, hit.line, hit.col, string_group, names_text(info)))
      end
    end

    if corpus_case.expectNoInterpolation then
      local line_nr, col = find_plain_string_column(lines, corpus_case.startLine, corpus_case.endLine)
      if not line_nr or not col then
        fail('syntax', string.format('case=%s file=%s reason=no_plain_string_probe', corpus_case.id, corpus_case.materializedPath))
      end
      local info = syntax_info(line_nr, col)
      io.stdout:write(string.format(
        '[m036-s02] phase=syntax case=%s source=%s materialized=%s line=%d col=%d probe=plain-string %s\n',
        corpus_case.id,
        corpus_case.sourcePath,
        corpus_case.materializedPath,
        line_nr,
        col,
        names_text(info)
      ))
      if names_text(info) == '' then
        fail('syntax', string.format('case=%s file=%s line=%d col=%d reason=no_syntax_group', corpus_case.id, corpus_case.materializedPath, line_nr, col))
      end
      if has_group_prefix(info, 'meshInterpolation') then
        fail('syntax', string.format('case=%s file=%s line=%d col=%d reason=unexpected_interpolation_group %s', corpus_case.id, corpus_case.materializedPath, line_nr, col, names_text(info)))
      end
      if string_group and not has_group_prefix(info, string_group) then
        fail('syntax', string.format('case=%s file=%s line=%d col=%d reason=wrong_string_kind expected=%s %s', corpus_case.id, corpus_case.materializedPath, line_nr, col, string_group, names_text(info)))
      end
    end
  end

  local decorator_probe_count = run_cluster_decorator_syntax_case()
  local unicode_identifier_probe_count = run_unicode_identifier_boundary_syntax_case()
  local current_surface_probe_count = run_current_syntax_surface_case()
  local compiler_token_probe_count = run_compiler_token_syntax_case()

  io.stdout:write(string.format(
    '[m036-s02] phase=syntax result=pass checked_cases=%d decorator_probes=%d unicode_identifier_probes=%d current_surface_probes=%d compiler_token_probes=%d\n',
    #selected_cases,
    decorator_probe_count,
    unicode_identifier_probe_count,
    current_surface_probe_count,
    compiler_token_probe_count
  ))
end

local function mesh_clients(bufnr, include_uninitialized)
  local filter = { bufnr = bufnr, name = 'mesh' }
  if include_uninitialized then
    filter._uninitialized = true
  end
  return vim.lsp.get_clients(filter)
end

local function wait_for_mesh_client(bufnr, timeout_ms)
  local attached
  local ok = vim.wait(timeout_ms, function()
    for _, client in ipairs(mesh_clients(bufnr, false)) do
      if not client:is_stopped() then
        attached = client
        return true
      end
    end
    return false
  end, 50)

  return ok and attached or nil
end

local function current_mesh_error()
  local last_error = vim.g.mesh_lsp_last_error
  if type(last_error) == 'string' and last_error ~= '' then
    return last_error
  end
  return '<none>'
end

local function assert_missing_override_fails(sample_path)
  local missing = vim.fs.joinpath(repo_root, '.tmp', 'm036-s02', 'missing-meshc')
  local resolution, err = mesh.resolve_meshc({
    override = missing,
    buffer_path = sample_path,
    cwd = repo_root,
  })

  if resolution then
    fail('lsp', string.format('reason=missing_override_unexpected_success path=%s', missing))
  end

  if type(err) ~= 'string' or not err:find(missing, 1, true) then
    fail('lsp', string.format('reason=missing_override_missing_path path=%s error=%s', missing, tostring(err)))
  end

  if not err:find('vim.g.mesh_lsp_path', 1, true) then
    fail('lsp', string.format('reason=missing_override_missing_actionable_message error=%s', err))
  end

  io.stdout:write(string.format('[m036-s02] phase=lsp negative=missing-override result=pass path=%s\n', rel(missing)))
end

local function summarize_client(bufnr, client, label)
  local detected_root = mesh.detect_root(bufnr)
  local resolved = client.config._mesh_resolved or {}
  local root_dir = client.config.root_dir
  io.stdout:write(string.format(
    '[m036-s02] phase=lsp case=%s client=%s client_id=%d buffer=%s root=%s marker=%s marker_path=%s meshc_class=%s meshc_path=%s\n',
    label,
    client.name,
    client.id,
    rel(vim.api.nvim_buf_get_name(bufnr)),
    rel(root_dir),
    detected_root.marker,
    rel(detected_root.marker_path),
    resolved.class or '<unknown>',
    rel(resolved.path)
  ))
end

local function materialize_override_entry_project()
  local project_dir = vim.fs.joinpath(repo_root, '.tmp', 'm036-s02', requested_phase, 'override-entry-project')
  local manifest_path = vim.fs.joinpath(project_dir, 'mesh.toml')
  local entry_path = vim.fs.joinpath(project_dir, 'lib', 'start.mpl')
  local support_path = vim.fs.joinpath(project_dir, 'lib', 'support', 'message.mpl')

  ensure_clean_dir(project_dir)
  write_file(manifest_path, {
    '[package]',
    'name = "override-entry-project"',
    'version = "0.1.0"',
    'entrypoint = "lib/start.mpl"',
  })
  write_file(entry_path, {
    'from Lib.Support.Message import message',
    '',
    'fn main() do',
    '  let rendered = message()',
    '  println("proof=#{rendered}")',
    'end',
  })
  write_file(support_path, {
    'pub fn message() -> String do',
    '  "nested-support"',
    'end',
  })

  io.stdout:write(string.format(
    '[m036-s02] phase=lsp case=override-entry materialized project=%s manifest=%s entry=%s support=%s\n',
    rel(project_dir),
    rel(manifest_path),
    rel(entry_path),
    rel(support_path)
  ))

  return {
    project_dir = project_dir,
    manifest_path = manifest_path,
    entry_path = entry_path,
    support_path = support_path,
  }
end

local function assert_override_entry_project_attach()
  local fixture = materialize_override_entry_project()
  local entry_buf = open_buffer(fixture.entry_path)
  local entry_client = wait_for_mesh_client(entry_buf, 8000)
  if not entry_client then
    fail('lsp', string.format('reason=attach_timeout case=override-entry-entry buffer=%s root_marker=%s last_error=%s', rel(fixture.entry_path), mesh.detect_root(entry_buf).marker, current_mesh_error()))
  end
  summarize_client(entry_buf, entry_client, 'override-entry-entry')

  local detected_root = mesh.detect_root(entry_buf)
  if canonical(detected_root.root_dir) ~= canonical(fixture.project_dir)
    or detected_root.marker ~= 'mesh.toml'
    or canonical(detected_root.marker_path) ~= canonical(fixture.manifest_path)
  then
    fail('lsp', string.format(
      'reason=wrong_root case=override-entry expected_root=%s actual_root=%s expected_marker=mesh.toml actual_marker=%s expected_marker_path=%s actual_marker_path=%s',
      rel(fixture.project_dir),
      rel(detected_root.root_dir),
      detected_root.marker,
      rel(fixture.manifest_path),
      rel(detected_root.marker_path)
    ))
  end

  if canonical(entry_client.config.root_dir) ~= canonical(fixture.project_dir) then
    fail('lsp', string.format(
      'reason=wrong_client_root case=override-entry expected=%s actual=%s',
      rel(fixture.project_dir),
      rel(entry_client.config.root_dir)
    ))
  end

  local support_buf = open_buffer(fixture.support_path)
  local support_client = wait_for_mesh_client(support_buf, 8000)
  if not support_client then
    fail('lsp', string.format('reason=attach_timeout case=override-entry-support buffer=%s root_marker=%s last_error=%s', rel(fixture.support_path), mesh.detect_root(support_buf).marker, current_mesh_error()))
  end
  summarize_client(support_buf, support_client, 'override-entry-support')

  if support_client.id ~= entry_client.id then
    fail('lsp', string.format('reason=duplicate_client case=override-entry expected_reuse_of=%d actual=%d', entry_client.id, support_client.id))
  end
end

local function assert_real_project_reuses_client()
  local health_path = example_health_path
  local todos_path = example_todos_path

  local health_buf = open_buffer(health_path)
  local health_client = wait_for_mesh_client(health_buf, 8000)
  if not health_client then
    fail('lsp', string.format('reason=attach_timeout case=health buffer=%s root_marker=%s last_error=%s', rel(health_path), mesh.detect_root(health_buf).marker, current_mesh_error()))
  end
  summarize_client(health_buf, health_client, 'example-health')

  local health_root = canonical(health_client.config.root_dir)
  local expected_root = canonical(example_root)
  if health_root ~= expected_root then
    fail('lsp', string.format('reason=wrong_root case=health expected=%s actual=%s', rel(expected_root), rel(health_client.config.root_dir)))
  end

  local resolved = health_client.config._mesh_resolved or {}
  if type(resolved.path) ~= 'string' or resolved.path == '' then
    fail('lsp', 'reason=missing_resolved_meshc_path case=health')
  end
  if type(resolved.candidates) ~= 'table' or #resolved.candidates == 0 then
    fail('lsp', 'reason=missing_candidate_trace case=health')
  end

  local todos_buf = open_buffer(todos_path)
  local todos_client = wait_for_mesh_client(todos_buf, 8000)
  if not todos_client then
    fail('lsp', string.format('reason=attach_timeout case=todos buffer=%s root_marker=%s last_error=%s', rel(todos_path), mesh.detect_root(todos_buf).marker, current_mesh_error()))
  end
  summarize_client(todos_buf, todos_client, 'example-todos')

  if todos_client.id ~= health_client.id then
    fail('lsp', string.format('reason=duplicate_client expected_reuse_of=%d actual=%d', health_client.id, todos_client.id))
  end

  local mesh_count = #mesh_clients(todos_buf, false)
  if mesh_count ~= 1 then
    fail('lsp', string.format('reason=unexpected_client_count count=%d buffer=%s', mesh_count, rel(todos_path)))
  end
end

local function assert_single_file_attach()
  local temp_dir = vim.fn.tempname()
  if vim.fn.mkdir(temp_dir, 'p') ~= 1 then
    fail('lsp', string.format('reason=failed_to_create_tempdir path=%s', temp_dir))
  end

  local standalone_path = vim.fs.joinpath(temp_dir, 'standalone.mpl')
  local lines = {
    'fn greet(name :: String) -> String do',
    '  "hello #{name}"',
    'end',
    '',
    'let value = greet("mesh")',
  }
  vim.fn.writefile(lines, standalone_path)

  local bufnr = open_buffer(standalone_path)
  local client = wait_for_mesh_client(bufnr, 8000)
  if not client then
    fail('lsp', string.format('reason=attach_timeout case=standalone buffer=%s root_marker=%s last_error=%s', rel(standalone_path), mesh.detect_root(bufnr).marker, current_mesh_error()))
  end

  summarize_client(bufnr, client, 'standalone-file')

  local detected_root = mesh.detect_root(bufnr)
  if detected_root.root_dir ~= nil or detected_root.marker ~= 'single-file' then
    fail('lsp', string.format('reason=expected_single_file_mode actual_root=%s marker=%s', rel(detected_root.root_dir), detected_root.marker))
  end

  if client.config.root_dir ~= nil then
    fail('lsp', string.format('reason=standalone_root_should_be_nil actual=%s', rel(client.config.root_dir)))
  end

  local resolved = client.config._mesh_resolved or {}
  if type(resolved.path) ~= 'string' or resolved.path == '' then
    fail('lsp', 'reason=missing_resolved_meshc_path case=standalone')
  end
end

local function run_lsp_phase()
  local supported, support_error = mesh.assert_native_lsp()
  if not supported then
    fail('lsp', string.format('reason=unsupported_neovim %s', support_error))
  end

  if type(vim.lsp.enable) ~= 'function' then
    fail('lsp', 'reason=missing_vim_lsp_enable')
  end

  assert_missing_override_fails(example_health_path)
  assert_real_project_reuses_client()
  assert_override_entry_project_attach()
  assert_single_file_attach()

  io.stdout:write('[m036-s02] phase=lsp result=pass checked_cases=4\n')
end

if not run_syntax and not run_lsp then
  fail('smoke', string.format('unsupported smoke phase: %s', requested_phase))
end

if run_syntax then
  run_syntax_phase()
end

if run_lsp then
  run_lsp_phase()
end

vim.cmd('qa!')
