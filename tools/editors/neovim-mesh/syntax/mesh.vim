if exists('b:current_syntax')
  finish
endif

syntax case match
syntax sync fromstart

syntax cluster meshClosureHeaderTop contains=meshDocModuleComment,meshDocComment,meshComment,meshCommentBlock,meshRegex,meshAtom,meshStringTriple,meshStringDouble,meshNumberHex,meshNumberBinary,meshNumberOctal,meshNumberFloat,meshNumberInteger,meshPunctuation,meshVariable,meshType,meshFunctionCall,meshModuleCall,meshFromImport,meshImportDecl,meshFunctionDeclaration,meshHandlerDeclaration,meshTypeDeclaration,meshClusterDecorator,meshNativeDecorator,meshOrmKeyword,meshContextualKeyword,meshControlKeyword,meshDeclarationKeyword,meshWordOperator,meshBoolean,meshBuiltinType,meshBuiltinConstructor,meshWildcard,meshRangeOperator,meshDiamondOperator,meshConcatOperator,meshFatArrowOperator,meshTryOperator,meshLogicalAndOperator,meshLogicalOrOperator,meshPipeOperator,meshSlotPipeOperator,meshAnnotationOperator,meshComparisonOperator,meshResultOperator,meshBarOperator,meshArithmeticOperator,meshAssignmentOperator,meshMapDelimiter,meshInvalidSlotPipe
syntax cluster meshTopNoEnd contains=@meshClosureHeaderTop,meshDoKeyword,meshArrowOperator
syntax cluster meshTop contains=@meshTopNoEnd,meshEndKeyword

syntax match meshDocModuleComment /##!.*$/
syntax match meshDocComment /##[^!].*$/
syntax match meshDocComment /##$/
syntax match meshComment /#[^#=].*$/
syntax match meshComment /#$/
syntax region meshCommentBlock start=/#=/ end=/=#/ contains=meshCommentBlock

syntax match meshStringEscape /\\./ contained
syntax region meshInterpolation matchgroup=meshInterpolationDelimiter start=/#{/ end=/}/ keepend contained contains=meshInterpolationBrace,@meshTop
syntax region meshInterpolation matchgroup=meshInterpolationDelimiter start=/\${/ end=/}/ keepend contained contains=meshInterpolationBrace,@meshTop
syntax region meshInterpolationBrace matchgroup=meshInterpolationDelimiter start=/{/ end=/}/ keepend contained contains=meshInterpolationBrace,@meshTop
syntax region meshStringTriple start=/"""/ end=/"""/ keepend contains=meshStringEscape,meshInterpolation
syntax region meshStringDouble start=/\%("\)\@<!"\%("\)\@!/ skip=/\\\\\|\\"/ end=/\%("\)\@<!"\%("\)\@!/ keepend contains=meshStringEscape,meshInterpolation

syntax match meshNumberHex /\<0[xX][0-9a-fA-F_]\+\>/
syntax match meshNumberBinary /\<0[bB][01_]\+\>/
syntax match meshNumberOctal /\<0[oO][0-7_]\+\>/
syntax match meshNumberInteger /\<[0-9][0-9_]*\>/
syntax match meshNumberFloat /\v<[0-9][0-9_]*(\.[0-9][0-9_]*)?[eE][+-]?[0-9_]+>/
syntax match meshNumberFloat /\v<[0-9][0-9_]*\.[0-9][0-9_]*>/

syntax match meshPunctuation /[(),.;@]/
syntax match meshPunctuation /[{}\[\]]/
syntax match meshPunctuation /:/

" Vim has no Rust-equivalent Unicode Alphabetic/Alphanumeric properties.
" Treating every non-ASCII character as identifier-shaped is a deliberate
" coverage-first approximation; compiler-invalid Unicode punctuation may also
" receive identifier highlighting.
let s:mesh_nfa = '\%#=2'
let s:mesh_id_start = '\%([A-Za-z_]\|[^\x00-\x7f]\)'
let s:mesh_id_continue = '\%([0-9A-Za-z_]\|[^\x00-\x7f]\)'
let s:mesh_id_left = s:mesh_id_continue . '\@<!'
let s:mesh_id_right = s:mesh_id_continue . '\@!'
let s:mesh_identifier = s:mesh_id_start . s:mesh_id_continue . '*'
let s:mesh_value_id_start = '\%([a-z_]\|[^\x00-\x7f]\)'
let s:mesh_value_identifier = s:mesh_value_id_start . s:mesh_id_continue . '*'
" Vim's [:upper:] includes Unicode Nl characters such as U+2163, unlike Lu.
" Keep the lexical type heuristic ASCII-only; declaration contexts cover other types.
let s:mesh_type_identifier = '[A-Z]' . s:mesh_id_continue . '*'
let s:mesh_block_comment = '#=\%(\%(=#\)\@!\_.\)*=#'
let s:mesh_gap = '\%(\s\|' . s:mesh_block_comment . '\)\+'
let s:mesh_optional_gap = '\%(\s\|' . s:mesh_block_comment . '\)*'
let s:mesh_do_word = s:mesh_id_left . 'do' . s:mesh_id_right
let s:mesh_end_word = s:mesh_id_left . 'end' . s:mesh_id_right
let s:mesh_fn_word = s:mesh_id_left . 'fn' . s:mesh_id_right
let s:mesh_def_word = s:mesh_id_left . 'def' . s:mesh_id_right
let s:mesh_pub_word = s:mesh_id_left . 'pub' . s:mesh_id_right
let s:mesh_where_word = s:mesh_id_left . 'where' . s:mesh_id_right
let s:mesh_when_word = s:mesh_id_left . 'when' . s:mesh_id_right

execute 'syntax region meshRegex start=+' . s:mesh_nfa . '\~r/\ze\%(\_[^/\\]\|\\\_.\)*/[ims]*\%([A-Za-z]\)\@!+ skip=+' . s:mesh_nfa . '\\\_.+ end=+' . s:mesh_nfa . '/[ims]*\%([A-Za-z]\)\@!+ keepend'
execute 'syntax match meshAtom /' . s:mesh_nfa . ':[a-z_]' . s:mesh_id_continue . '*/'

execute 'syntax match meshVariable /' . s:mesh_nfa . s:mesh_id_left . s:mesh_identifier . s:mesh_id_right . '/'
execute 'syntax match meshType /' . s:mesh_nfa . s:mesh_id_left . s:mesh_type_identifier . s:mesh_id_right . '/'
execute 'syntax match meshVariable /' . s:mesh_nfa . '\%(' . s:mesh_id_left . 'let' . s:mesh_id_right . s:mesh_gap . '\)\@<=' . s:mesh_identifier . s:mesh_id_right . '/'
execute 'syntax match meshFunctionCall /' . s:mesh_nfa . s:mesh_id_left . '\%(\%(do\|end\)' . s:mesh_id_right . '\)\@!' . s:mesh_identifier . s:mesh_id_right . '\ze\s*(/'

execute 'syntax match meshModuleCall /' . s:mesh_nfa . s:mesh_id_left . s:mesh_type_identifier . '\%(\.' . s:mesh_type_identifier . '\)*\.' . s:mesh_identifier . s:mesh_id_right . '\ze\s*(/ contains=meshModuleAccessor,meshModuleFunctionCall'
syntax match meshModuleAccessor /\./ contained
execute 'syntax match meshModuleFunctionCall /' . s:mesh_nfa . s:mesh_id_left . s:mesh_identifier . s:mesh_id_right . '\ze\s*(/ contained'

execute 'syntax match meshFromImport /' . s:mesh_nfa . s:mesh_id_left . 'from' . s:mesh_id_right . s:mesh_gap . s:mesh_identifier . '\%(\.' . s:mesh_identifier . '\)*' . s:mesh_gap . s:mesh_id_left . 'import' . s:mesh_id_right . '/ transparent contains=meshImportKeyword,meshModulePath'
execute 'syntax match meshImportDecl /' . s:mesh_nfa . s:mesh_id_left . 'import' . s:mesh_id_right . s:mesh_gap . s:mesh_identifier . '\%(\.' . s:mesh_identifier . '\)*' . s:mesh_id_right . '/ transparent contains=meshImportKeyword,meshModulePath'
execute 'syntax match meshModulePath /' . s:mesh_nfa . s:mesh_id_left . s:mesh_type_identifier . '\%(\.' . s:mesh_type_identifier . '\)*' . s:mesh_id_right . '/ contained contains=meshModuleAccessor'
execute 'syntax match meshImportKeyword /' . s:mesh_nfa . s:mesh_id_left . '\%(from\|import\)' . s:mesh_id_right . '/ contained'

execute 'syntax match meshClusterDecorator /' . s:mesh_nfa . '@\s*cluster' . s:mesh_id_right . '/'
execute 'syntax match meshNativeDecorator /' . s:mesh_nfa . '@\s*native' . s:mesh_id_right . '\ze\s*(/'

execute 'syntax match meshOrmKeyword /' . s:mesh_nfa . '\%(' . s:mesh_end_word . s:mesh_gap . '\)\@<=' . s:mesh_id_left . 'deriving' . s:mesh_id_right . '\ze' . s:mesh_optional_gap . '(/'
execute 'syntax match meshOrmKeyword /' . s:mesh_nfa . s:mesh_id_left . 'table' . s:mesh_id_right . '\ze' . s:mesh_gap . '"/'
execute 'syntax match meshOrmKeyword /' . s:mesh_nfa . s:mesh_id_left . 'primary_key' . s:mesh_id_right . '\ze' . s:mesh_gap . ':[a-z_]' . s:mesh_id_continue . '*/'
execute 'syntax match meshOrmKeyword /' . s:mesh_nfa . s:mesh_id_left . 'timestamps' . s:mesh_id_right . '\ze' . s:mesh_gap . s:mesh_id_left . '\%(true\|false\)' . s:mesh_id_right . '/'
execute 'syntax match meshOrmKeyword /' . s:mesh_nfa . s:mesh_id_left . '\%(belongs_to\|has_many\|has_one\)' . s:mesh_id_right . '\ze' . s:mesh_gap . ':[a-z_]' . s:mesh_id_continue . '*' . s:mesh_optional_gap . ',/'

execute 'syntax match meshContextualKeyword /' . s:mesh_nfa . s:mesh_id_left . 'as' . s:mesh_id_right . '\ze' . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . '/'

execute 'syntax match meshControlKeyword /' . s:mesh_nfa . s:mesh_id_left . '\%(if\|else\|case\|match\|when\|return\|import\|for\|while\|cond\|break\|continue\)' . s:mesh_id_right . '/'
execute 'syntax match meshDoKeyword /' . s:mesh_nfa . s:mesh_do_word . '/'
execute 'syntax match meshEndKeyword /' . s:mesh_nfa . s:mesh_end_word . '/'
execute 'syntax match meshDeclarationKeyword /' . s:mesh_nfa . s:mesh_id_left . '\%(fn\|let\|def\|type\|struct\|module\|interface\|impl\|pub\|actor\|service\|supervisor\|call\|cast\|trait\|alias\|json\)' . s:mesh_id_right . '/'
execute 'syntax match meshWordOperator /' . s:mesh_nfa . s:mesh_id_left . '\%(and\|or\|not\|in\|where\|with\|spawn\|send\|receive\|self\|link\|monitor\|terminate\|trap\|after\)' . s:mesh_id_right . '/'
execute 'syntax match meshBoolean /' . s:mesh_nfa . s:mesh_id_left . '\%(true\|false\|nil\)' . s:mesh_id_right . '/'
execute 'syntax match meshBuiltinType /' . s:mesh_nfa . s:mesh_id_left . '\%(Atom\|Bool\|BootstrapStatus\|Bytes\|ContinuityAuthorityStatus\|ContinuityRecord\|ContinuitySubmitDecision\|DateTime\|Float\|Fun\|HttpClientMetrics\|HttpResponse\|I128\|Int\|Json\|List\|ListIterator\|Map\|MapIterator\|Never\|Option\|Ordering\|PgConn\|Pid\|PoolHandle\|Queue\|Range\|RangeIterator\|Regex\|Request\|Response\|Result\|Router\|Set\|SetIterator\|SqliteConn\|String\|Tuple\|U128\|U64\|Unit\|WsMessage\)' . s:mesh_id_right . '/'
execute 'syntax match meshBuiltinConstructor /' . s:mesh_nfa . s:mesh_id_left . '\%(Some\|None\|Ok\|Err\|Less\|Equal\|Greater\)' . s:mesh_id_right . '/'
execute 'syntax match meshWildcard /' . s:mesh_nfa . s:mesh_id_left . '_' . s:mesh_id_right . '/'

let s:mesh_declaration_tail = '\%(<\|(\|->\|' . s:mesh_where_word . '\|' . s:mesh_when_word . '\|=\|' . s:mesh_do_word . '\)'
execute 'syntax match meshFunctionDeclaration /' . s:mesh_nfa . s:mesh_pub_word . s:mesh_gap . s:mesh_id_left . '\%(fn\|def\)' . s:mesh_id_right . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . '\ze' . s:mesh_optional_gap . s:mesh_declaration_tail . '/ contains=meshDeclarationKeyword,meshFunctionName'
execute 'syntax match meshFunctionDeclaration /' . s:mesh_nfa . '^\s*\zs' . s:mesh_def_word . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . '\ze' . s:mesh_optional_gap . s:mesh_declaration_tail . '/ contains=meshDeclarationKeyword,meshCommentBlock,meshFunctionName'
execute 'syntax match meshFunctionDeclaration /' . s:mesh_nfa . '^\s*\zs' . s:mesh_fn_word . s:mesh_gap . s:mesh_value_identifier . s:mesh_id_right . '\ze' . s:mesh_optional_gap . '(/ contains=meshDeclarationKeyword,meshCommentBlock,meshFunctionName'
execute 'syntax match meshFunctionDeclaration /' . s:mesh_nfa . '^\s*\zs' . s:mesh_fn_word . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . '\ze' . s:mesh_optional_gap . '\%(<\|=\)/ contains=meshDeclarationKeyword,meshCommentBlock,meshFunctionName'
execute 'syntax match meshFunctionName /' . s:mesh_nfa . s:mesh_id_left . s:mesh_identifier . s:mesh_id_right . '\ze' . s:mesh_optional_gap . s:mesh_declaration_tail . '/ contained'
execute 'syntax match meshFunctionName /' . s:mesh_nfa . s:mesh_id_left . s:mesh_identifier . s:mesh_id_right . '\ze\s*\%($\|;\|#\)/ contained'
execute 'syntax match meshHandlerDeclaration /' . s:mesh_nfa . s:mesh_id_left . '\%(call\|cast\)' . s:mesh_id_right . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . '\ze' . s:mesh_optional_gap . '\%((\|::\|' . s:mesh_do_word . '\)/ contains=meshDeclarationKeyword,meshCommentBlock,meshHandlerName'
execute 'syntax match meshHandlerName /' . s:mesh_nfa . s:mesh_id_left . s:mesh_identifier . s:mesh_id_right . '\ze' . s:mesh_optional_gap . '\%((\|::\|' . s:mesh_do_word . '\)/ contained'
let s:mesh_type_declaration_word = s:mesh_id_left . '\%(module\|struct\|type\|actor\|service\|supervisor\|interface\)' . s:mesh_id_right
execute 'syntax match meshTypeDeclaration /' . s:mesh_nfa . s:mesh_type_declaration_word . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . '/ contains=meshDeclarationKeyword,meshCommentBlock,meshTypeDeclarationName'
execute 'syntax match meshTypeDeclarationName /' . s:mesh_nfa . '\%(' . s:mesh_type_declaration_word . s:mesh_gap . '\)\@<=' . s:mesh_identifier . s:mesh_id_right . '/ contained'

execute 'syntax match meshInterfaceMethodDeclaration /' . s:mesh_nfa . '^\s*\zs' . s:mesh_id_left . '\%(fn\|def\)' . s:mesh_id_right . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . '/ contained transparent contains=meshDeclarationKeyword,meshCommentBlock,meshFunctionName'
execute 'syntax match meshInterfaceHeader /' . s:mesh_nfa . '\%(' . s:mesh_pub_word . s:mesh_gap . '\)\?' . s:mesh_id_left . 'interface' . s:mesh_id_right . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . '\%(' . s:mesh_optional_gap . '<.\{-}>\)\?' . s:mesh_gap . '\ze' . s:mesh_do_word . '/ transparent contains=@meshTop nextgroup=meshInterfaceBlock skipwhite'
execute 'syntax region meshInterfaceBlock matchgroup=meshControlKeyword start=/' . s:mesh_nfa . s:mesh_do_word . '/ end=/' . s:mesh_nfa . s:mesh_end_word . '/ contained transparent contains=@meshTop,meshInterfaceMethodDeclaration,meshDoBlock'

syntax match meshArithmeticOperator /[+*%\/]/
syntax match meshArithmeticOperator /-\ze\%([^>]\|$\)/
syntax match meshAssignmentOperator /=/
syntax match meshResultOperator /!\ze\%([^=]\|$\)/
syntax match meshBarOperator /|\ze\%([^|>0-9]\|$\)/
syntax match meshComparisonOperator /==\|!=\|<=\|>=\|<\|\%(-\)\@<!>/
syntax match meshDiamondOperator /\V<>/
syntax match meshConcatOperator /\V++/
syntax match meshFatArrowOperator /\V=>/
syntax match meshLogicalAndOperator /\V&&/
syntax match meshLogicalOrOperator /\V||/
syntax match meshPipeOperator /\V|>/
syntax match meshSlotPipeOperator /\v\|0*([2-9]|[1-9][0-9]+)>/
syntax match meshArrowOperator /\V->/
syntax match meshAnnotationOperator /\V::/
syntax match meshRangeOperator /\.\./
syntax match meshTryOperator /\V?/
syntax match meshMapDelimiter /%{/
syntax match meshInvalidSlotPipe /\v\|0*[01]>/
syntax match meshInvalidSlotPipe /|[0-9]\+\ze\%([^0-9>]\|$\)/

execute 'syntax match meshSupervisorKeywordTop /' . s:mesh_nfa . s:mesh_id_left . '\%(strategy\|max_restarts\|max_seconds\)' . s:mesh_id_right . '\ze' . s:mesh_optional_gap . ':/ contained'
execute 'syntax match meshSupervisorValueTop /' . s:mesh_nfa . s:mesh_id_left . '\%(one_for_one\|one_for_all\|rest_for_one\|simple_one_for_one\)' . s:mesh_id_right . '/ contained'
execute 'syntax match meshSupervisorClauseTop /' . s:mesh_nfa . s:mesh_id_left . 'strategy' . s:mesh_id_right . s:mesh_optional_gap . ':' . s:mesh_optional_gap . s:mesh_id_left . '\%(one_for_one\|one_for_all\|rest_for_one\|simple_one_for_one\)' . s:mesh_id_right . '/ contained transparent contains=meshSupervisorKeywordTop,meshSupervisorValueTop,meshPunctuation,meshCommentBlock'

execute 'syntax match meshSupervisorKeywordChild /' . s:mesh_nfa . s:mesh_id_left . '\%(start\|restart\|shutdown\)' . s:mesh_id_right . '\ze' . s:mesh_optional_gap . ':/ contained'
execute 'syntax match meshSupervisorKeywordChild /' . s:mesh_nfa . s:mesh_id_left . 'child' . s:mesh_id_right . '\ze' . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . s:mesh_gap . s:mesh_do_word . '/ contained'
execute 'syntax match meshSupervisorValueChild /' . s:mesh_nfa . s:mesh_id_left . '\%(permanent\|transient\|temporary\|brutal_kill\)' . s:mesh_id_right . '/ contained'
execute 'syntax match meshSupervisorClauseChild /' . s:mesh_nfa . s:mesh_id_left . 'restart' . s:mesh_id_right . s:mesh_optional_gap . ':' . s:mesh_optional_gap . s:mesh_id_left . '\%(permanent\|transient\|temporary\)' . s:mesh_id_right . '/ contained transparent contains=meshSupervisorKeywordChild,meshSupervisorValueChild,meshPunctuation,meshCommentBlock'
execute 'syntax match meshSupervisorClauseChild /' . s:mesh_nfa . s:mesh_id_left . 'shutdown' . s:mesh_id_right . s:mesh_optional_gap . ':' . s:mesh_optional_gap . s:mesh_id_left . 'brutal_kill' . s:mesh_id_right . '/ contained transparent contains=meshSupervisorKeywordChild,meshSupervisorValueChild,meshPunctuation,meshCommentBlock'

let s:mesh_named_function_suffix = '\%(' . s:mesh_gap . s:mesh_value_identifier . s:mesh_id_right . s:mesh_optional_gap . '(\|' . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . s:mesh_optional_gap . '=\)'
execute 'syntax region meshClosureHeader matchgroup=meshDeclarationKeyword start=/' . s:mesh_nfa . s:mesh_fn_word . '\%(' . s:mesh_named_function_suffix . '\)\@!/ matchgroup=meshEndKeyword end=/' . s:mesh_nfa . s:mesh_end_word . '/ contained transparent contains=@meshTopNoEnd,meshClosureSignature,meshClosureHeader,meshElseCandidate,meshDoBlock'
execute 'syntax region meshClosureSignature start=/' . s:mesh_nfa . '\%(' . s:mesh_fn_word . '\)\@<=\%(' . s:mesh_named_function_suffix . '\)\@!/ matchgroup=meshClosureArrow end=/->/ matchgroup=meshClosureDo end=/' . s:mesh_nfa . s:mesh_do_word . '/ contained transparent contains=@meshClosureHeaderTop,meshClosureHeader,meshClosureFunType,meshClosureGuardBlock,meshClosureReceiveGuardBlock'
execute 'syntax region meshClosureFunType start=/' . s:mesh_nfa . s:mesh_id_left . 'Fun' . s:mesh_id_right . '\ze' . s:mesh_optional_gap . '(/ matchgroup=meshArrowOperator end=/->/ contained transparent contains=@meshClosureHeaderTop,meshClosureFunType'
let s:mesh_guard_word = s:mesh_id_left . '\%(if\|case\|match\|while\|for\)' . s:mesh_id_right
let s:mesh_receive_word = s:mesh_id_left . 'receive' . s:mesh_id_right
execute 'syntax region meshClosureGuardBlock matchgroup=meshControlKeyword start=/' . s:mesh_nfa . s:mesh_guard_word . '/ matchgroup=meshEndKeyword end=/' . s:mesh_nfa . s:mesh_end_word . '/ contained transparent contains=@meshTopNoEnd,meshClosureGuardCondition,meshClosureHeader,meshElseCandidate,meshDoBlock'
execute 'syntax region meshClosureReceiveGuardBlock matchgroup=meshWordOperator start=/' . s:mesh_nfa . s:mesh_receive_word . '/ matchgroup=meshEndKeyword end=/' . s:mesh_nfa . s:mesh_end_word . '/ contained transparent contains=@meshTopNoEnd,meshClosureGuardCondition,meshClosureHeader,meshElseCandidate,meshDoBlock'
execute 'syntax region meshClosureGuardCondition start=/' . s:mesh_nfa . '\%(' . s:mesh_id_left . '\%(if\|case\|match\|while\|for\|receive\)' . s:mesh_id_right . '\)\@<=/ matchgroup=meshDoKeyword end=/' . s:mesh_nfa . s:mesh_do_word . '/ contained transparent contains=@meshClosureHeaderTop,meshClosureHeader,meshClosureFunType,meshClosureGuardBlock,meshClosureReceiveGuardBlock'
execute 'syntax region meshDoBlock matchgroup=meshDoKeyword start=/' . s:mesh_nfa . s:mesh_do_word . '/ matchgroup=meshEndKeyword end=/' . s:mesh_nfa . s:mesh_end_word . '/ contained transparent contains=@meshTopNoEnd,meshElseCandidate,meshClosureHeader,meshDoBlock'
execute 'syntax region meshElseIfCondition matchgroup=meshControlKeyword start=/' . s:mesh_nfa . s:mesh_id_left . 'if' . s:mesh_id_right . '/ matchgroup=meshDoKeyword end=/' . s:mesh_nfa . s:mesh_do_word . '/ contained transparent contains=@meshClosureHeaderTop,meshClosureHeader,meshClosureFunType,meshClosureGuardBlock,meshClosureReceiveGuardBlock'
execute 'syntax match meshElseCandidate /' . s:mesh_nfa . s:mesh_id_left . 'else' . s:mesh_id_right . '/ contained nextgroup=meshElseIfCondition,meshElseIfModuleComment,meshElseIfDocComment,meshElseIfComment,meshElseIfCommentBlock skipwhite skipnl skipempty'
syntax match meshElseIfModuleComment /##!.*$/ contained nextgroup=meshElseIfCondition,meshElseIfModuleComment,meshElseIfDocComment,meshElseIfComment,meshElseIfCommentBlock skipwhite skipnl skipempty
syntax match meshElseIfDocComment /##[^!].*$/ contained nextgroup=meshElseIfCondition,meshElseIfModuleComment,meshElseIfDocComment,meshElseIfComment,meshElseIfCommentBlock skipwhite skipnl skipempty
syntax match meshElseIfDocComment /##$/ contained nextgroup=meshElseIfCondition,meshElseIfModuleComment,meshElseIfDocComment,meshElseIfComment,meshElseIfCommentBlock skipwhite skipnl skipempty
syntax match meshElseIfComment /#[^#=].*$/ contained nextgroup=meshElseIfCondition,meshElseIfModuleComment,meshElseIfDocComment,meshElseIfComment,meshElseIfCommentBlock skipwhite skipnl skipempty
syntax match meshElseIfComment /#$/ contained nextgroup=meshElseIfCondition,meshElseIfModuleComment,meshElseIfDocComment,meshElseIfComment,meshElseIfCommentBlock skipwhite skipnl skipempty
syntax region meshElseIfCommentBlock matchgroup=meshElseIfCommentBlock start=/#=/ end=/=#/ contained contains=meshCommentBlock nextgroup=meshElseIfCondition,meshElseIfModuleComment,meshElseIfDocComment,meshElseIfComment,meshElseIfCommentBlock skipwhite skipnl skipempty

execute 'syntax match meshSupervisorChildHeader /' . s:mesh_nfa . s:mesh_id_left . 'child' . s:mesh_id_right . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . s:mesh_gap . '\ze' . s:mesh_do_word . '/ contained transparent contains=@meshTop,meshSupervisorKeywordChild nextgroup=meshSupervisorChildBlock skipwhite'
execute 'syntax region meshSupervisorChildBlock matchgroup=meshControlKeyword start=/' . s:mesh_nfa . s:mesh_do_word . '/ end=/' . s:mesh_nfa . s:mesh_end_word . '/ contained transparent contains=@meshTop,meshSupervisorClauseChild,meshSupervisorKeywordChild,meshClosureHeader,meshDoBlock'
execute 'syntax match meshSupervisorHeader /' . s:mesh_nfa . '\%(' . s:mesh_pub_word . s:mesh_gap . '\)\?' . s:mesh_id_left . 'supervisor' . s:mesh_id_right . s:mesh_gap . s:mesh_identifier . s:mesh_id_right . s:mesh_gap . '\ze' . s:mesh_do_word . '/ transparent contains=@meshTop nextgroup=meshSupervisorBlock skipwhite'
execute 'syntax region meshSupervisorBlock matchgroup=meshControlKeyword start=/' . s:mesh_nfa . s:mesh_do_word . '/ end=/' . s:mesh_nfa . s:mesh_end_word . '/ contained transparent contains=@meshTop,meshSupervisorClauseTop,meshSupervisorKeywordTop,meshSupervisorChildHeader'

highlight default link meshDocModuleComment SpecialComment
highlight default link meshDocComment SpecialComment
highlight default link meshComment Comment
highlight default link meshCommentBlock Comment
highlight default link meshStringTriple String
highlight default link meshStringDouble String
highlight default link meshStringEscape SpecialChar
highlight default link meshInterpolation Special
highlight default link meshInterpolationDelimiter Delimiter
highlight default link meshNumberHex Number
highlight default link meshNumberBinary Number
highlight default link meshNumberOctal Number
highlight default link meshNumberFloat Float
highlight default link meshNumberInteger Number
highlight default link meshPunctuation Delimiter
highlight default link meshClusterDecorator PreProc
highlight default link meshNativeDecorator PreProc
highlight default link meshRegex String
highlight default link meshAtom Constant
highlight default link meshModulePath Include
highlight default link meshImportKeyword Include
highlight default link meshModuleCall Identifier
highlight default link meshModuleAccessor Delimiter
highlight default link meshModuleFunctionCall Function
highlight default link meshFunctionCall Function
highlight default link meshFunctionDeclaration Function
highlight default link meshFunctionName Function
highlight default link meshHandlerDeclaration Function
highlight default link meshHandlerName Function
highlight default link meshTypeDeclaration Type
highlight default link meshTypeDeclarationName Type
highlight default link meshOrmKeyword Keyword
highlight default link meshSupervisorKeywordTop Keyword
highlight default link meshSupervisorKeywordChild Keyword
highlight default link meshSupervisorValueTop Constant
highlight default link meshSupervisorValueChild Constant
highlight default link meshContextualKeyword Keyword
highlight default link meshControlKeyword Conditional
highlight default link meshDoKeyword Conditional
highlight default link meshEndKeyword Conditional
highlight default link meshClosureArrow Operator
highlight default link meshClosureDo Conditional
highlight default link meshElseCandidate Conditional
highlight default link meshElseIfModuleComment SpecialComment
highlight default link meshElseIfDocComment SpecialComment
highlight default link meshElseIfComment Comment
highlight default link meshElseIfCommentBlock Comment
highlight default link meshDeclarationKeyword Keyword
highlight default link meshWordOperator Operator
highlight default link meshBoolean Boolean
highlight default link meshBuiltinType Type
highlight default link meshBuiltinConstructor Function
highlight default link meshType Type
highlight default link meshWildcard Special
highlight default link meshRangeOperator Operator
highlight default link meshDiamondOperator Operator
highlight default link meshConcatOperator Operator
highlight default link meshFatArrowOperator Operator
highlight default link meshTryOperator Operator
highlight default link meshLogicalAndOperator Operator
highlight default link meshLogicalOrOperator Operator
highlight default link meshPipeOperator Operator
highlight default link meshSlotPipeOperator Operator
highlight default link meshArrowOperator Operator
highlight default link meshAnnotationOperator Operator
highlight default link meshComparisonOperator Operator
highlight default link meshResultOperator Operator
highlight default link meshBarOperator Operator
highlight default link meshArithmeticOperator Operator
highlight default link meshAssignmentOperator Operator
highlight default link meshMapDelimiter Delimiter
highlight default link meshInvalidSlotPipe Error
highlight default link meshVariable Identifier

let b:current_syntax = 'mesh'
