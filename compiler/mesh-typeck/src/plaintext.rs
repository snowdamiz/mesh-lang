//! The plaintext checker: where `Plaintext` values may go.
//!
//! `Plaintext<T>` is a nominal type with no trait impls, so no sink accepts
//! one: printing, logging, panics, interpolation, JSON, HTTP, WebSockets,
//! files and host callbacks all take other types, and a type holding one
//! derives nothing (`infer.rs`). This pass adds what types alone do not say:
//!
//! - `declassify(value, "reason")` is called directly, with a non-empty
//!   string literal for its reason; each call is a reported site.
//! - Only functions marked `@display` export plaintext to the host, and
//!   `@display` marks an `@export` that carries some.
//! - `Plaintext.map`/`map2` apply a function that has no exits (see
//!   "Purity" below), written at the call or named, so what they compute on
//!   the content can only come back labeled.
//! - Plaintext goes to the program's own actors only: not to an untyped
//!   `Pid`, not through a conversion between an untyped `Pid` and a pid for
//!   plaintext messages, not to `Node.spawn` or `@cluster` work.
//!
//! See `docs/security/plaintext-types.md`.

use mesh_parser::ast::expr::{CallExpr, Expr, FieldAccess, NameRef, StringExpr};
use mesh_parser::ast::item::{FnDef, ImplDef};
use mesh_parser::ast::AstNode;
use mesh_parser::{Parse, SyntaxKind, SyntaxNode};
use rowan::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::{ConstraintOrigin, TypeError};
use crate::infer::TypeRegistry;
use crate::ty::{Ty, PLAINTEXT};
use crate::ImportContext;

/// A `declassify` call: where, in which function, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclassifySite {
    pub function: String,
    pub reason: String,
    pub span: TextRange,
}

/// An `@display` export: the function and the symbol the host calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayExport {
    pub function: String,
    pub symbol: String,
    pub span: TextRange,
}

/// What a module says about plaintext, beyond its errors.
#[derive(Debug, Default, Clone)]
pub struct PlaintextFacts {
    pub declassify_sites: Vec<DeclassifySite>,
    pub display_exports: Vec<DisplayExport>,
    /// Top-level functions with no exits, which `Plaintext.map` may apply.
    pub pure_functions: FxHashSet<String>,
}

pub(crate) struct PlaintextCheck {
    pub errors: Vec<TypeError>,
    pub facts: PlaintextFacts,
}

/// Standard-library modules none of whose functions is a way out: they
/// compute. (A value they dispatch on is checked apart: `dispatch_safe`.)
const PURE_MODULES: &[&str] = &[
    "String",
    "Bytes",
    "BytesBuilder",
    "List",
    "Map",
    "Set",
    "Tuple",
    "Range",
    "Queue",
    "Iter",
    "Option",
    "Result",
    "Int",
    "Float",
    "Math",
    "U64",
    "U128",
    "I128",
    "Checked",
    "Base64",
    "Hex",
    "Json",
    "Regex",
    "Crypto",
    "DateTime",
    "Duration",
    "Monotonic",
    "Random",
    PLAINTEXT,
];

/// Types whose trait impls (`Display`, `Eq`, `Hash`, `ToJson`, ...) are the
/// compiler's own: a value of one dispatches to no code of the program.
const BUILTIN_TYPES: &[&str] = &[
    "Int",
    "Float",
    "Bool",
    "String",
    "Bytes",
    "U64",
    "U128",
    "I128",
    "Atom",
    "Json",
    "Regex",
    "DateTime",
    "BytesBuilder",
    "()",
    "List",
    "Map",
    "Set",
    "Option",
    "Result",
    "Queue",
    "Iter",
    "Range",
    PLAINTEXT,
];

pub(crate) fn check(
    parse: &Parse,
    types: &FxHashMap<TextRange, Ty>,
    registry: &TypeRegistry,
    import_ctx: &ImportContext,
    stdlib_imports: &FxHashMap<String, String>,
    untyped_pid_conversions: Vec<(Ty, ConstraintOrigin)>,
) -> PlaintextCheck {
    let root = parse.syntax();
    let mut checker = Checker {
        types,
        registry,
        import_ctx,
        stdlib_imports,
        top_level: top_level_functions(&root),
        imported: imported_functions(import_ctx),
        errors: Vec::new(),
        facts: PlaintextFacts::default(),
    };
    checker.facts.pure_functions = checker.pure_functions();
    checker.check_names(&root);
    for node in root.descendants() {
        match node.kind() {
            SyntaxKind::NAME_REF => checker.check_name_ref(&NameRef::cast(node).unwrap()),
            SyntaxKind::FIELD_ACCESS => {
                checker.check_field_access(&FieldAccess::cast(node).unwrap())
            }
            SyntaxKind::SEND_EXPR => checker.check_send(&node),
            SyntaxKind::CALL_EXPR => checker.check_comparison(&CallExpr::cast(node).unwrap()),
            SyntaxKind::FN_DEF => checker.check_boundary(&FnDef::cast(node).unwrap()),
            _ => {}
        }
    }
    // A map or set keyed by plaintext compares keys on every lookup, and
    // its answers are not labeled. Every map value has a type: the first
    // one, in the source, is reported.
    // (A module that names no plaintext type has none.)
    let may_hold_plaintext =
        !registry.plaintext_types.is_empty() || root.to_string().contains(PLAINTEXT);
    if let Some((span, ty)) = types
        .iter()
        .filter(|_| may_hold_plaintext)
        .filter(|(_, ty)| checker.keyed_by_plaintext(ty))
        .min_by_key(|(span, _)| (span.start(), span.end()))
    {
        checker.violation(
            format!(
                "`{ty}` is keyed by plaintext: a lookup compares keys, and what it answers is \
                 not labeled"
            ),
            *span,
        );
    }
    let mut seen = FxHashSet::default();
    for (message, origin) in untyped_pid_conversions {
        let span = origin.span().unwrap_or_default();
        if registry.is_plaintext_type(&message)
            && !hands_the_pid_to_the_runtime(&root, &origin)
            && seen.insert(span)
        {
            checker.violation(
                format!(
                    "a pid for plaintext messages (`Pid<{message}>`) and an untyped `Pid` do not \
                     convert: an untyped pid may name an actor on another node"
                ),
                span,
            );
        }
    }
    PlaintextCheck {
        errors: checker.errors,
        facts: checker.facts,
    }
}

/// The error for a trait `ty`, which holds plaintext, lacks: what the
/// trait would have done with the content.
pub(crate) fn missing_trait(ty: &Ty, trait_name: &str, origin: &ConstraintOrigin) -> TypeError {
    let what = match trait_name {
        "Display" | "Debug" => "shown or interpolated into a string".to_string(),
        "Eq" | "Ord" => {
            "compared (compare inside `Plaintext.map2`, which keeps the answer labeled)".to_string()
        }
        "Hash" => "hashed".to_string(),
        "Json" | "ToJson" | "FromJson" | "FromRow" | "Row" | "Schema" => "serialized".to_string(),
        other => format!("given to `{other}`"),
    };
    TypeError::PlaintextViolation {
        reason: format!(
            "`{}` holds plaintext, which cannot be {what}",
            ty.with_holes()
        ),
        span: origin.span().unwrap_or_default(),
    }
}

/// Whether a conversion to an untyped `Pid` is the pid given to
/// `Process.register`, `Global.register` or `Process.monitor`: the runtime
/// keeps it, and hands back only untyped pids, to which no plaintext goes.
fn hands_the_pid_to_the_runtime(root: &SyntaxNode, origin: &ConstraintOrigin) -> bool {
    let ConstraintOrigin::FnArg {
        call_site,
        param_idx,
    } = origin
    else {
        return false;
    };
    let call = match root.covering_element(*call_site) {
        rowan::NodeOrToken::Node(node) => node.ancestors().find_map(CallExpr::cast),
        rowan::NodeOrToken::Token(token) => token.parent_ancestors().find_map(CallExpr::cast),
    };
    let Some(Expr::FieldAccess(access)) = call.and_then(|call| call.callee()) else {
        return false;
    };
    let Some(Expr::NameRef(module)) = access.base() else {
        return false;
    };
    let function = access.field().map(|field| field.text().to_string());
    matches!(
        (module.text().as_deref(), function.as_deref(), param_idx),
        (Some("Process"), Some("monitor"), 0) | (Some("Process" | "Global"), Some("register"), 1)
    )
}

struct Checker<'a> {
    types: &'a FxHashMap<TextRange, Ty>,
    registry: &'a TypeRegistry,
    import_ctx: &'a ImportContext,
    stdlib_imports: &'a FxHashMap<String, String>,
    /// The module's top-level functions, each name's clauses and arities.
    top_level: FxHashMap<String, Vec<FnDef>>,
    /// The functions imported modules export, by name (an overloaded one's
    /// arities under one), and whether each has no exits.
    imported: FxHashMap<String, bool>,
    errors: Vec<TypeError>,
    facts: PlaintextFacts,
}

impl Checker<'_> {
    fn violation(&mut self, reason: String, span: TextRange) {
        self.errors
            .push(TypeError::PlaintextViolation { reason, span });
    }

    fn ty(&self, node: &SyntaxNode) -> Option<&Ty> {
        self.types.get(&node.text_range())
    }

    fn holds_plaintext(&self, node: &SyntaxNode) -> bool {
        self.ty(node)
            .is_some_and(|ty| self.registry.is_plaintext_type(ty))
    }

    /// `declassify` and `Plaintext` are the language's: a program defines
    /// neither, nor reaches `Plaintext`'s functions by another name.
    fn check_names(&mut self, root: &SyntaxNode) {
        for node in root.descendants() {
            let defines = |name: &str| {
                node.children()
                    .find(|child| child.kind() == SyntaxKind::NAME)
                    .is_some_and(|child| child.text() == name)
                    || matches!(node.kind(), SyntaxKind::PARAM | SyntaxKind::IDENT_PAT)
                        && node
                            .children_with_tokens()
                            .filter_map(|element| element.into_token())
                            .any(|token| token.kind() == SyntaxKind::IDENT && token.text() == name)
            };
            match node.kind() {
                SyntaxKind::FN_DEF
                | SyntaxKind::PARAM
                | SyntaxKind::LET_BINDING
                | SyntaxKind::IDENT_PAT
                    if defines("declassify") =>
                {
                    self.violation(
                        "`declassify` is the language's declassification and cannot be redefined"
                            .to_string(),
                        node.text_range(),
                    );
                }
                SyntaxKind::STRUCT_DEF
                | SyntaxKind::SUM_TYPE_DEF
                | SyntaxKind::MODULE_DEF
                | SyntaxKind::TYPE_ALIAS_DEF
                | SyntaxKind::INTERFACE_DEF
                    if defines(PLAINTEXT) =>
                {
                    self.violation(
                        "`Plaintext` is the language's content type and cannot be redefined"
                            .to_string(),
                        node.text_range(),
                    );
                }
                SyntaxKind::FROM_IMPORT_DECL | SyntaxKind::IMPORT_DECL => {
                    let path = node
                        .children()
                        .find(|child| child.kind() == SyntaxKind::PATH)
                        .map(|path| path.text().to_string())
                        .unwrap_or_default();
                    let last = path.rsplit('.').next().unwrap_or_default().trim();
                    if last == PLAINTEXT {
                        self.violation(
                            "`Plaintext`'s functions are reached as `Plaintext.map`, which the \
                             checker follows; no module or import may stand in for them"
                                .to_string(),
                            node.text_range(),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// A reference to `declassify`: a direct call, with a reason written
    /// in the source.
    fn check_name_ref(&mut self, name_ref: &NameRef) {
        if name_ref.text().as_deref() != Some("declassify") {
            return;
        }
        // A binding of that name is refused already; its uses are its own.
        if is_bound_in(&enclosing_scope(name_ref.syntax()), name_ref) {
            return;
        }
        let Some(call) = direct_call(name_ref.syntax()) else {
            self.violation(
                "`declassify` is called directly: `declassify(value, \"reason\")`".to_string(),
                name_ref.syntax().text_range(),
            );
            return;
        };
        let span = call.syntax().text_range();
        let args = call.args();
        let [_, reason] = args.as_slice() else {
            self.violation(
                "`declassify` takes the value and a reason: \
                 `declassify(value, \"why it may be disclosed\")`"
                    .to_string(),
                span,
            );
            return;
        };
        let Some(reason) = string_literal(reason) else {
            self.violation(
                "the reason given to `declassify` must be a string literal, which the build \
                 report lists"
                    .to_string(),
                reason.syntax().text_range(),
            );
            return;
        };
        if reason.trim().is_empty() {
            self.violation(
                "the reason given to `declassify` must not be empty".to_string(),
                span,
            );
            return;
        }
        self.facts.declassify_sites.push(DeclassifySite {
            function: enclosing_function(call.syntax()),
            reason,
            span,
        });
    }

    fn check_field_access(&mut self, access: &FieldAccess) {
        let Some(Expr::NameRef(base)) = access.base() else {
            return;
        };
        let field = access.field().map(|token| token.text().to_string());
        match (base.text().as_deref(), field.as_deref()) {
            (Some(PLAINTEXT), Some(name @ ("from" | "map" | "map2"))) => {
                let Some(call) = direct_call(access.syntax()) else {
                    self.violation(
                        format!(
                            "`Plaintext.{name}` is called directly: `Plaintext.{name}(...)`, \
                             not piped into or passed as a value"
                        ),
                        access.syntax().text_range(),
                    );
                    return;
                };
                if name != "from" {
                    self.check_lifted(&call, name);
                }
            }
            (Some("Node"), Some(name @ ("spawn" | "spawn_link"))) => {
                let Some((node, args)) = effective_call(access.syntax()) else {
                    return;
                };
                if self.holds_plaintext(&node)
                    || args.iter().any(|arg| self.holds_plaintext(arg.syntax()))
                {
                    self.violation(
                        format!(
                            "`Node.{name}` starts an actor on another node: its arguments and \
                             messages cannot hold plaintext"
                        ),
                        node.text_range(),
                    );
                }
            }
            _ => {}
        }
    }

    /// `Plaintext.map(value, f)` or `map2(a, b, f)`: `f` has no exits.
    fn check_lifted(&mut self, call: &CallExpr, name: &str) {
        let Some(function) = call.args().last().cloned() else {
            return;
        };
        let span = function.syntax().text_range();
        let problem = match &function {
            Expr::ClosureExpr(closure) => {
                let scope = enclosing_scope(closure.syntax());
                self.impurity(closure.syntax(), &scope)
                    .map(|found| found.reason)
            }
            Expr::NameRef(named) if !is_bound_in(&enclosing_scope(named.syntax()), named) => {
                let function = named.text().unwrap_or_default();
                if self.top_level.contains_key(&function) {
                    self.function_impurity(&function)
                        .map(|reason| format!("`{function}` has an exit: {reason}"))
                } else if let Some(pure) = self.imported_purity(&function) {
                    (!pure).then(|| format!("`{function}` has an exit"))
                } else {
                    Some(not_checkable(name))
                }
            }
            Expr::FieldAccess(access) => match self.module_function(access) {
                Some((module, function, Some(pure))) => {
                    (!pure).then(|| format!("`{module}.{function}` has an exit"))
                }
                _ => Some(not_checkable(name)),
            },
            _ => Some(not_checkable(name)),
        };
        if let Some(reason) = problem {
            let reason = if reason.starts_with("the function") {
                reason
            } else {
                format!("`Plaintext.{name}` needs a function with no exits: {reason}")
            };
            self.violation(reason, span);
        }
    }

    /// `M.f` of an imported module: its names and whether `f` has no exits.
    fn module_function(&self, access: &FieldAccess) -> Option<(String, String, Option<bool>)> {
        let Some(Expr::NameRef(base)) = access.base() else {
            return None;
        };
        let module = base.text()?;
        let function = access.field()?.text().to_string();
        let exports = self.import_ctx.module_exports.get(&module)?;
        let pure = exports.pure_functions.contains(&function);
        Some((module, function, Some(pure)))
    }

    /// Whether a bare name imported from another module (`from M import f`)
    /// has no exits; `None` when no imported module exports it.
    fn imported_purity(&self, function: &str) -> Option<bool> {
        self.imported.get(function).copied()
    }

    /// Whether `ty` is or holds a `Map` or `Set` keyed by plaintext.
    fn keyed_by_plaintext(&self, ty: &Ty) -> bool {
        let keyed = ty
            .args_of("Map")
            .or_else(|| ty.args_of("Set"))
            .and_then(<[Ty]>::first)
            .is_some_and(|key| self.registry.is_plaintext_type(key));
        keyed || ty.parts().any(|part| self.keyed_by_plaintext(part))
    }

    /// `contains(collection, value)` of plaintext compares it, and what it
    /// answers is not labeled.
    fn check_comparison(&mut self, call: &CallExpr) {
        let Some(Expr::FieldAccess(access)) = call.callee() else {
            return;
        };
        if access
            .field()
            .is_none_or(|field| field.text() != "contains")
        {
            return;
        }
        let Some((node, args)) = effective_call(access.syntax()) else {
            return;
        };
        let receiver = access.base().filter(|base| {
            !matches!(base, Expr::NameRef(name) if name.text().is_some_and(|name| {
                crate::infer::is_stdlib_module(&name)
            }))
        });
        if args
            .iter()
            .chain(receiver.as_ref())
            .any(|arg| self.holds_plaintext(arg.syntax()))
        {
            self.violation(
                "`contains` compares plaintext, and what it answers is not labeled: compare \
                 inside `Plaintext.map2`"
                    .to_string(),
                node.text_range(),
            );
        }
    }

    /// `send(pid, message)`: a message holding plaintext goes to a typed pid.
    fn check_send(&mut self, send: &SyntaxNode) {
        let Some((node, args)) = effective_call(send) else {
            return;
        };
        let [pid, message] = args.as_slice() else {
            return;
        };
        let untyped = self
            .ty(pid.syntax())
            .is_some_and(|ty| matches!(ty, Ty::Con(con) if con.name == "Pid"));
        if untyped && self.holds_plaintext(message.syntax()) {
            self.violation(
                "a message holding plaintext goes only to a typed `Pid<M>` of the program's own \
                 actors (from `spawn`, `self()` or a service's `start`), not to an untyped `Pid`"
                    .to_string(),
                node.text_range(),
            );
        }
    }

    /// An exported or clustered function: what crosses its boundary.
    fn check_boundary(&mut self, function: &FnDef) {
        let name = function
            .name()
            .and_then(|name| name.text())
            .unwrap_or_default();
        let span = function.syntax().text_range();
        // A function's type is plaintext by what it takes and gives.
        let carries = match self.ty(function.syntax()) {
            Some(Ty::Fun(params, result)) => params
                .iter()
                .chain(std::iter::once(result.as_ref()))
                .any(|ty| self.registry.is_plaintext_type(ty)),
            _ => false,
        };
        match (function.export_decl(), function.display_decl()) {
            (Some(export), Some(_)) if carries => {
                self.facts.display_exports.push(DisplayExport {
                    function: name.clone(),
                    symbol: export.symbol().unwrap_or_default(),
                    span: function
                        .name()
                        .map_or(span, |name| name.syntax().text_range()),
                });
            }
            (Some(_), Some(display)) => self.violation(
                format!("`{name}` carries no plaintext: remove `@display`"),
                display.syntax().text_range(),
            ),
            (Some(_), None) if carries => self.violation(
                format!(
                    "`{name}` carries plaintext across the library boundary: mark it `@display` \
                     (the build report lists every such export)"
                ),
                span,
            ),
            (None, Some(display)) => self.violation(
                "`@display` marks an `@export` that carries plaintext to the host".to_string(),
                display.syntax().text_range(),
            ),
            _ => {}
        }
        if function.clustered_decl().is_some() && carries {
            self.violation(
                format!(
                    "`{name}` runs on other nodes (`@cluster`): its arguments and result cannot \
                     hold plaintext"
                ),
                span,
            );
        }
    }
}

/// The message for a function argument of `Plaintext.map` the checker
/// cannot look into.
fn not_checkable(name: &str) -> String {
    format!(
        "the function `Plaintext.{name}` applies must be a closure written here or a named \
         function of the program, which the checker can see has no exits (wrap a library \
         function: `fn(x) -> String.trim(x) end`)"
    )
}

/// The call `node` (a callee) is made in, when it is called directly: the
/// callee of a call that is no pipe's right-hand side.
fn direct_call(node: &SyntaxNode) -> Option<CallExpr> {
    let call = node.parent().and_then(CallExpr::cast)?;
    let is_callee = call.callee().is_some_and(|callee| callee.syntax() == node);
    let piped = call.syntax().parent().is_some_and(|parent| {
        matches!(
            parent.kind(),
            SyntaxKind::PIPE_EXPR | SyntaxKind::SLOT_PIPE_EXPR
        ) && parent.children().nth(1).as_ref() == Some(call.syntax())
    });
    (is_callee && !piped).then_some(call)
}

/// The expression calling `callee` and its arguments, a pipe's value among
/// them where it goes (`x |> f(a)` is `f(x, a)`, `x |2> f(a)` is `f(a, x)`).
fn effective_call(callee: &SyntaxNode) -> Option<(SyntaxNode, Vec<Expr>)> {
    let (node, mut args) = match callee.parent() {
        Some(parent) if parent.kind() == SyntaxKind::CALL_EXPR => {
            let call = CallExpr::cast(parent.clone())?;
            if call.callee().is_some_and(|c| c.syntax() == callee) {
                (parent, call.args())
            } else {
                return None;
            }
        }
        _ if callee.kind() == SyntaxKind::SEND_EXPR => {
            let args = callee
                .children()
                .find(|child| child.kind() == SyntaxKind::ARG_LIST)
                .map(|list| list.children().filter_map(Expr::cast).collect())
                .unwrap_or_default();
            (callee.clone(), args)
        }
        Some(parent)
            if matches!(
                parent.kind(),
                SyntaxKind::PIPE_EXPR | SyntaxKind::SLOT_PIPE_EXPR
            ) =>
        {
            (callee.clone(), Vec::new())
        }
        _ => return None,
    };
    // Piped into: the pipe gives its value at the pipe's slot.
    let Some(pipe) = node.parent().filter(|parent| {
        matches!(
            parent.kind(),
            SyntaxKind::PIPE_EXPR | SyntaxKind::SLOT_PIPE_EXPR
        ) && parent.children().nth(1).as_ref() == Some(&node)
    }) else {
        return Some((node, args));
    };
    let lhs = pipe.children().find_map(Expr::cast);
    let slot = match pipe.kind() {
        SyntaxKind::SLOT_PIPE_EXPR => mesh_parser::ast::expr::SlotPipeExpr::cast(pipe.clone())
            .and_then(|slot_pipe| slot_pipe.slot())
            .map_or(0, |slot| slot as usize - 1),
        _ => 0,
    };
    if let Some(lhs) = lhs {
        args.insert(slot.min(args.len()), lhs);
    }
    Some((pipe, args))
}

/// The text of a string literal without interpolation.
fn string_literal(expr: &Expr) -> Option<String> {
    let Expr::StringExpr(string) = expr else {
        return None;
    };
    literal_text(string)
}

fn literal_text(string: &StringExpr) -> Option<String> {
    if string
        .syntax()
        .children()
        .any(|child| child.kind() == SyntaxKind::INTERPOLATION)
    {
        return None;
    }
    Some(
        string
            .syntax()
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .filter(|token| {
                !matches!(
                    token.kind(),
                    SyntaxKind::STRING_START | SyntaxKind::STRING_END
                )
            })
            .map(|token| token.text().to_string())
            .collect(),
    )
}

/// The function, actor or service `node` is in, as the build report names
/// it: `name`, or `Type.name` in an impl.
fn enclosing_function(node: &SyntaxNode) -> String {
    for ancestor in node.ancestors() {
        if !matches!(
            ancestor.kind(),
            SyntaxKind::FN_DEF | SyntaxKind::ACTOR_DEF | SyntaxKind::SERVICE_DEF
        ) {
            continue;
        }
        let Some(name) = ancestor
            .children()
            .find(|child| child.kind() == SyntaxKind::NAME)
            .map(|name| name.text().to_string())
        else {
            continue;
        };
        let owner = ancestor
            .ancestors()
            .find_map(ImplDef::cast)
            .and_then(|impl_def| impl_def.type_name())
            .map(|owner| format!("{}.", owner.text()));
        return format!("{}{name}", owner.unwrap_or_default());
    }
    "<top level>".to_string()
}

/// The functions `import_ctx`'s modules export, by name without an arity
/// suffix (`name__2`), and whether they have no exits.
fn imported_functions(import_ctx: &ImportContext) -> FxHashMap<String, bool> {
    let mut imported = FxHashMap::default();
    for exports in import_ctx.module_exports.values() {
        for name in exports.functions.keys() {
            let base = name
                .rsplit_once("__")
                .filter(|(_, arity)| arity.parse::<u32>().is_ok())
                .map_or(name.as_str(), |(base, _)| base);
            imported.insert(base.to_string(), exports.pure_functions.contains(base));
        }
    }
    imported
}

/// The module's top-level functions by name, each with its clauses and
/// arities (an `@native` one is native code, with no body to check).
fn top_level_functions(root: &SyntaxNode) -> FxHashMap<String, Vec<FnDef>> {
    let mut functions: FxHashMap<String, Vec<FnDef>> = FxHashMap::default();
    for function in root.children().filter_map(FnDef::cast) {
        if let Some(name) = function.name().and_then(|name| name.text()) {
            functions.entry(name).or_default().push(function);
        }
    }
    functions
}

// ── Purity ─────────────────────────────────────────────────────────────

/// What makes code have an exit, and where.
struct Impurity {
    reason: String,
}

/// The names bound in the function a checked piece of code is in:
/// parameters and every pattern, anywhere in it. `closures` are those bound
/// only by `let name = fn ... end` inside the checked code, whose bodies the
/// walk sees; any other function value is code it cannot see.
struct Scope {
    bound: FxHashSet<String>,
    closures: FxHashSet<String>,
}

/// The scope of `checked`: the names of the function (or actor, service or
/// impl) it is in, and the closures it binds itself.
fn enclosing_scope(checked: &SyntaxNode) -> Scope {
    let item = checked
        .ancestors()
        .filter(|ancestor| {
            matches!(
                ancestor.kind(),
                SyntaxKind::FN_DEF
                    | SyntaxKind::ACTOR_DEF
                    | SyntaxKind::SERVICE_DEF
                    | SyntaxKind::IMPL_DEF
            )
        })
        .last()
        .unwrap_or_else(|| checked.clone());
    // Each binding of a name, and whether it binds a closure written in
    // `checked`.
    let mut bindings: Vec<(String, bool)> = Vec::new();
    for descendant in item.descendants() {
        match descendant.kind() {
            // A parameter's name is its own token.
            SyntaxKind::PARAM => bindings.extend(
                descendant
                    .children_with_tokens()
                    .filter_map(|element| element.into_token())
                    .filter(|token| token.kind() == SyntaxKind::IDENT)
                    .map(|token| (token.text().to_string(), false)),
            ),
            SyntaxKind::IDENT_PAT => bindings.extend(
                descendant
                    .children_with_tokens()
                    .filter_map(|element| element.into_token())
                    .filter(|token| token.kind() == SyntaxKind::IDENT)
                    .map(|token| {
                        let in_closure_let = descendant.parent().is_some_and(|parent| {
                            parent.kind() == SyntaxKind::LET_BINDING
                                && parent
                                    .children()
                                    .any(|child| child.kind() == SyntaxKind::CLOSURE_EXPR)
                                && checked.text_range().contains_range(parent.text_range())
                        });
                        (token.text().to_string(), in_closure_let)
                    }),
            ),
            SyntaxKind::NAME
                if descendant.parent().is_some_and(|parent| {
                    !matches!(
                        parent.kind(),
                        SyntaxKind::FN_DEF
                            | SyntaxKind::ACTOR_DEF
                            | SyntaxKind::SERVICE_DEF
                            | SyntaxKind::STRUCT_DEF
                            | SyntaxKind::SUM_TYPE_DEF
                            | SyntaxKind::MODULE_DEF
                            | SyntaxKind::STRUCT_FIELD
                            | SyntaxKind::STRUCT_LITERAL_FIELD
                    )
                }) =>
            {
                let in_closure_let = descendant.parent().is_some_and(|parent| {
                    parent.kind() == SyntaxKind::LET_BINDING
                        && parent
                            .children()
                            .any(|child| child.kind() == SyntaxKind::CLOSURE_EXPR)
                        && checked.text_range().contains_range(parent.text_range())
                });
                bindings.push((descendant.text().to_string(), in_closure_let));
            }
            _ => {}
        }
    }
    let bound: FxHashSet<String> = bindings.iter().map(|(name, _)| name.clone()).collect();
    let closures = bound
        .iter()
        .filter(|name| {
            bindings
                .iter()
                .filter(|(bound_name, _)| bound_name == *name)
                .all(|(_, closure)| *closure)
        })
        .cloned()
        .collect();
    Scope { bound, closures }
}

fn is_bound_in(scope: &Scope, name: &NameRef) -> bool {
    name.text().is_some_and(|text| scope.bound.contains(&text))
}

impl Checker<'_> {
    /// The top-level functions with no exits: each is, unless its body has
    /// one or calls one that has, until no more are found.
    fn pure_functions(&self) -> FxHashSet<String> {
        let mut pure: FxHashSet<String> = self.top_level.keys().cloned().collect();
        let mut direct: FxHashMap<String, (Option<Impurity>, FxHashSet<String>)> =
            FxHashMap::default();
        for (name, clauses) in &self.top_level {
            let mut calls = FxHashSet::default();
            let mut found = None;
            for clause in clauses {
                if clause.native_decl().is_some() {
                    found.get_or_insert(Impurity {
                        reason: "it is native code".to_string(),
                    });
                    continue;
                }
                let scope = enclosing_scope(clause.syntax());
                let mut walk = Walk {
                    checker: self,
                    scope: &scope,
                    calls: &mut calls,
                };
                if let Some(impurity) = walk.first_exit(clause.syntax()) {
                    found.get_or_insert(impurity);
                }
            }
            direct.insert(name.clone(), (found, calls));
        }
        loop {
            let impure: Vec<String> = pure
                .iter()
                .filter(|name| {
                    let (found, calls) = &direct[*name];
                    found.is_some() || calls.iter().any(|call| !pure.contains(call))
                })
                .cloned()
                .collect();
            if impure.is_empty() {
                break;
            }
            for name in impure {
                pure.remove(&name);
            }
        }
        pure
    }

    /// Why the top-level function `name` has an exit, if it has one.
    fn function_impurity(&self, name: &str) -> Option<String> {
        if self.facts.pure_functions.contains(name) {
            return None;
        }
        let clauses = self.top_level.get(name)?;
        for clause in clauses {
            if clause.native_decl().is_some() {
                return Some("it is native code".to_string());
            }
            let scope = enclosing_scope(clause.syntax());
            let mut calls = FxHashSet::default();
            let mut walk = Walk {
                checker: self,
                scope: &scope,
                calls: &mut calls,
            };
            if let Some(impurity) = walk.first_exit(clause.syntax()) {
                return Some(impurity.reason);
            }
            if let Some(call) = calls
                .iter()
                .find(|call| !self.facts.pure_functions.contains(*call))
            {
                return Some(format!("it calls `{call}`, which has an exit"));
            }
        }
        None
    }

    /// The first exit in `node` (a closure written at a `Plaintext.map`),
    /// with the program's functions it calls checked too.
    fn impurity(&self, node: &SyntaxNode, scope: &Scope) -> Option<Impurity> {
        let mut calls = FxHashSet::default();
        let mut walk = Walk {
            checker: self,
            scope,
            calls: &mut calls,
        };
        let found = walk.first_exit(node);
        found.or_else(|| {
            let mut calls: Vec<&String> = calls.iter().collect();
            calls.sort();
            calls
                .into_iter()
                .find(|call| !self.facts.pure_functions.contains(*call))
                .map(|call| Impurity {
                    reason: format!("it calls `{call}`, which has an exit"),
                })
        })
    }
}

/// A walk over code for its first exit, noting the module's own functions
/// it calls (whose exits the fixed point in `pure_functions` finds).
struct Walk<'c, 'a> {
    checker: &'c Checker<'a>,
    scope: &'c Scope,
    calls: &'c mut FxHashSet<String>,
}

impl Walk<'_, '_> {
    fn first_exit(&mut self, node: &SyntaxNode) -> Option<Impurity> {
        // Source order, so the reason given is the first exit written.
        for descendant in node.descendants() {
            if let Some(reason) = self.exit_at(&descendant) {
                return Some(Impurity { reason });
            }
        }
        None
    }

    fn exit_at(&mut self, node: &SyntaxNode) -> Option<String> {
        match node.kind() {
            SyntaxKind::SEND_EXPR => Some("it sends a message".to_string()),
            SyntaxKind::SPAWN_EXPR => Some("it spawns an actor".to_string()),
            SyntaxKind::RECEIVE_EXPR => Some("it receives messages".to_string()),
            SyntaxKind::SELF_EXPR => Some("it uses `self()`".to_string()),
            SyntaxKind::LINK_EXPR => Some("it links actors".to_string()),
            SyntaxKind::NAME_REF => self.name_exit(&NameRef::cast(node.clone())?),
            SyntaxKind::FIELD_ACCESS => self.field_exit(&FieldAccess::cast(node.clone())?),
            SyntaxKind::CALL_EXPR => {
                let callee = CallExpr::cast(node.clone())?.callee()?;
                (!matches!(callee, Expr::NameRef(_) | Expr::FieldAccess(_)))
                    .then(|| "it calls a function value the checker cannot see".to_string())
            }
            SyntaxKind::BINARY_EXPR | SyntaxKind::UNARY_EXPR => node
                .children()
                .filter_map(Expr::cast)
                .find_map(|operand| self.dispatch_exit(operand.syntax(), "applies an operator to")),
            SyntaxKind::INTERPOLATION => node
                .children()
                .filter_map(Expr::cast)
                .find_map(|inner| self.dispatch_exit(inner.syntax(), "interpolates")),
            SyntaxKind::JSON_EXPR => node
                .descendants()
                .filter(|field| field.kind() == SyntaxKind::JSON_FIELD)
                .flat_map(|field| field.children().filter_map(Expr::cast).collect::<Vec<_>>())
                .find_map(|value| self.dispatch_exit(value.syntax(), "encodes")),
            SyntaxKind::FOR_IN_EXPR => {
                let iterable = mesh_parser::ast::expr::ForInExpr::cast(node.clone())?.iterable()?;
                self.dispatch_exit(iterable.syntax(), "iterates over")
            }
            _ => None,
        }
    }

    /// A value of a type whose trait impls may be the program's own code.
    fn dispatch_exit(&self, value: &SyntaxNode, doing: &str) -> Option<String> {
        let ty = self.checker.ty(value)?;
        (!dispatch_safe(ty)).then(|| {
            format!(
                "it {doing} a value of type `{ty}`, whose trait impls may be code with exits \
                 (inside `Plaintext.map` only the built-in types' are known)"
            )
        })
    }

    fn name_exit(&mut self, name_ref: &NameRef) -> Option<String> {
        let name = name_ref.text()?;
        // A module's or type's name: the field access it starts is checked.
        if name_ref
            .syntax()
            .parent()
            .is_some_and(|parent| parent.kind() == SyntaxKind::FIELD_ACCESS)
            && name_ref.syntax().prev_sibling().is_none()
            && !self.scope.bound.contains(&name)
        {
            return None;
        }
        if name.starts_with(|c: char| c.is_ascii_uppercase()) || name == "declassify" {
            return None;
        }
        let is_function = matches!(self.checker.ty(name_ref.syntax()), Some(Ty::Fun(..)));
        if self.checker.top_level.contains_key(&name) {
            self.calls.insert(name.clone());
            if !self.scope.bound.contains(&name) {
                return None;
            }
        }
        if self.scope.bound.contains(&name) {
            return (is_function && !self.scope.closures.contains(&name))
                .then(|| format!("it calls `{name}`, a function value the checker cannot see"));
        }
        if let Some(pure) = self.checker.imported_purity(&name) {
            return (!pure).then(|| format!("it calls `{name}`, which has an exit"));
        }
        if let Some(module) = self.checker.stdlib_imports.get(&name) {
            if !PURE_MODULES.contains(&module.as_str()) {
                return Some(format!("it calls `{module}.{name}`"));
            }
            return self.dispatched_arguments(name_ref.syntax(), module, &name);
        }
        Some(format!("it calls `{name}`"))
    }

    fn field_exit(&mut self, access: &FieldAccess) -> Option<String> {
        let field = access.field()?.text().to_string();
        let base = access.base()?;
        let call = access
            .syntax()
            .parent()
            .and_then(CallExpr::cast)
            .filter(|call| call.callee().is_some_and(|c| c.syntax() == access.syntax()));
        if let Expr::NameRef(base_name) = &base {
            let module = base_name.text()?;
            if !self.scope.bound.contains(&module) {
                return self.qualified_exit(access, &module, &field, call.as_ref());
            }
        }
        // `value.method(...)`: a method of a built-in type is its module's.
        if call.is_some() {
            let receiver = self.checker.ty(base.syntax())?;
            let module = crate::infer::method_module(receiver);
            if module.is_none() || !dispatch_safe(receiver) {
                return Some(format!(
                    "it calls the method `{field}` of a `{receiver}`, which may be code with exits"
                ));
            }
            let call = call?;
            return call
                .args()
                .iter()
                .find_map(|arg| self.dispatch_exit(arg.syntax(), "passes"));
        }
        // A field holding a function is code the checker cannot see.
        matches!(self.checker.ty(access.syntax()), Some(Ty::Fun(..)))
            .then(|| format!("it takes the function in the field `{field}`, which it cannot see"))
    }

    /// `Module.function`, `Type.Variant` or `Service.handler`.
    fn qualified_exit(
        &self,
        access: &FieldAccess,
        module: &str,
        field: &str,
        call: Option<&CallExpr>,
    ) -> Option<String> {
        if field.starts_with(|c: char| c.is_ascii_uppercase()) {
            return None;
        }
        if let Some(exports) = self.checker.import_ctx.module_exports.get(module) {
            return (!exports.pure_functions.contains(field))
                .then(|| format!("it calls `{module}.{field}`, which has an exit"));
        }
        if !crate::infer::is_stdlib_module(module) {
            return Some(format!("it calls `{module}.{field}`"));
        }
        if !PURE_MODULES.contains(&module) {
            return Some(format!("it calls `{module}.{field}`"));
        }
        let _ = call;
        self.dispatched_arguments(access.syntax(), module, field)
    }

    /// A standard-library function (at `callee`) given, or taking, a value
    /// that may dispatch to the program's own trait impls.
    fn dispatched_arguments(
        &self,
        callee: &SyntaxNode,
        module: &str,
        function: &str,
    ) -> Option<String> {
        if never_dispatches(module, function) {
            return None;
        }
        let doing = format!("passes `{module}.{function}`");
        // Called, or piped into: what it is given.
        if let Some((_, args)) = effective_call(callee) {
            return args
                .iter()
                .find_map(|arg| self.dispatch_exit(arg.syntax(), &doing));
        }
        // Passed as a value (`List.map(notes, Json.encode)`): what it takes.
        match self.checker.ty(callee) {
            Some(Ty::Fun(params, _)) => params.iter().find(|ty| !dispatch_safe(ty)).map(|ty| {
                format!(
                    "it {doing} a value of type `{ty}`, whose trait impls may be code with exits \
                     (inside `Plaintext.map` only the built-in types' are known)"
                )
            }),
            _ => None,
        }
    }
}

/// Standard-library functions that only move values and call the closures
/// they are given, dispatching on nothing.
fn never_dispatches(module: &str, function: &str) -> bool {
    matches!(module, "Option" | "Result" | PLAINTEXT)
        || module == "List"
            && matches!(
                function,
                "map"
                    | "filter"
                    | "fold"
                    | "reduce"
                    | "flat_map"
                    | "filter_map"
                    | "find"
                    | "any"
                    | "all"
                    | "count"
                    | "length"
                    | "is_empty"
                    | "head"
                    | "tail"
                    | "last"
                    | "first"
                    | "get"
                    | "append"
                    | "prepend"
                    | "push"
                    | "concat"
                    | "reverse"
                    | "take"
                    | "drop"
                    | "zip"
                    | "enumerate"
                    | "new"
                    | "range"
                    | "repeat"
            )
}

/// Whether a value of `ty` dispatches only to the compiler's own trait
/// impls: built-in types all the way down. A function value is checked where
/// it is written or named.
fn dispatch_safe(ty: &Ty) -> bool {
    match ty {
        Ty::Con(con) => BUILTIN_TYPES.contains(&con.name.as_str()),
        Ty::App(head, args) => {
            matches!(head.as_ref(), Ty::Con(con) if BUILTIN_TYPES.contains(&con.name.as_str()))
                && args.iter().all(dispatch_safe)
        }
        Ty::Tuple(elements) => elements.iter().all(dispatch_safe),
        Ty::Fun(..) | Ty::Never => true,
        Ty::Var(_) => false,
    }
}
