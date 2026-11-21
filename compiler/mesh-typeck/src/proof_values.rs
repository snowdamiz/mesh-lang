//! Native proof values and theorem applications from embedded logic blocks.

use mesh_common::span::Span;
use mesh_parser::ast::item::SourceFile;
use mesh_parser::logic::{self, Binary, Binder, Declaration, Term};
use mesh_parser::{AstNode, SyntaxKind, SyntaxNode};
use rowan::{TextRange, TextSize};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::TypeError;
use crate::ty::{NatIndex, Ty, TyCon};
use crate::ImportContext;

pub(crate) const PROOF_TYPE: &str = "Proof";
const PROOF_HEAD: &str = "\0Proof";
const PREDICATE_PREFIX: &str = "\0proof predicate ";
const NAT_EQ: &str = "NatEq";
const NAT_EQ_HEAD: &str = "\0NatEq";
const NAT_TERM_PREFIX: &str = "\0proof Nat term ";
// A tuple of source-order slots followed by the proof result. Each Nat slot's
// parameter name is the substitution key in the premise and result types.
const CALLABLE_THEOREM_HEAD: &str = "\0callable theorem";

impl Ty {
    /// Whether this is the canonical type of a proof value.
    pub fn is_proof(&self) -> bool {
        matches!(self, Self::App(head, args) if args.len() == 1
            && matches!(head.as_ref(), Self::Con(con) if con.name == PROOF_HEAD))
    }

    /// Whether the type itself, or any of its parts, is a proof type.
    pub fn contains_proof(&self) -> bool {
        self.is_proof() || self.parts().any(Self::contains_proof)
    }

    /// Whether this type marks a theorem that can only appear as a direct call.
    pub fn is_callable_theorem(&self) -> bool {
        self.args_of(CALLABLE_THEOREM_HEAD).is_some()
    }

    pub(crate) fn callable_theorem(&self) -> Option<CallableTheorem<'_>> {
        let [Self::Tuple(params), result] = self.args_of(CALLABLE_THEOREM_HEAD)? else {
            return None;
        };
        if !result.is_proof()
            || !params
                .iter()
                .all(|param| matches!(param, Self::Nat(NatIndex::Parameter(_))) || param.is_proof())
        {
            return None;
        }
        Some(CallableTheorem { params, result })
    }
}

pub(crate) enum CallableTheoremParam<'a> {
    Nat(&'a str),
    Evidence(&'a Ty),
}

pub(crate) struct CallableTheorem<'a> {
    params: &'a [Ty],
    pub result: &'a Ty,
}

impl<'a> CallableTheorem<'a> {
    pub(crate) fn params(&self) -> impl ExactSizeIterator<Item = CallableTheoremParam<'a>> + 'a {
        self.params.iter().map(|param| match param {
            Ty::Nat(NatIndex::Parameter(name)) => CallableTheoremParam::Nat(name),
            proof => CallableTheoremParam::Evidence(proof),
        })
    }
}

/// Instantiate a theorem premise or result with the static Nat call arguments.
pub(crate) fn instantiate_theorem_proof(
    template: &Ty,
    indices: &FxHashMap<String, NatIndex>,
) -> Option<Ty> {
    fn instantiate_index(
        index: &NatIndex,
        indices: &FxHashMap<String, NatIndex>,
    ) -> Option<NatIndex> {
        match index {
            NatIndex::Literal(value) => Some(NatIndex::Literal(*value)),
            NatIndex::Parameter(name) => indices.get(name).cloned(),
            NatIndex::Succ(inner) => Some(instantiate_index(inner, indices)?.successor()),
            NatIndex::Variable(_) => None,
        }
    }
    fn instantiate_term(term: &Ty, indices: &FxHashMap<String, NatIndex>) -> Option<Ty> {
        match term {
            Ty::Nat(index) => instantiate_index(index, indices).map(Ty::Nat),
            Ty::App(head, args) => {
                let Ty::Con(head) = head.as_ref() else {
                    return None;
                };
                let operation = NatOperation::from_name(head.name.strip_prefix(NAT_TERM_PREFIX)?)?;
                let args = args
                    .iter()
                    .map(|arg| instantiate_term(arg, indices))
                    .collect::<Option<Vec<_>>>()?;
                operation.apply(args)
            }
            _ => None,
        }
    }

    let (head, args) = proposition_parts(template)?;
    let args = args
        .iter()
        .map(|arg| instantiate_term(arg, indices))
        .collect::<Option<Vec<_>>>()?;
    Some(proof_type(head.clone(), args))
}

/// The readable spelling of a compiler-owned proof constructor.
pub(crate) fn display_head(name: &str) -> Option<&str> {
    if name == PROOF_HEAD {
        Some(PROOF_TYPE)
    } else if name == NAT_EQ_HEAD {
        Some(NAT_EQ)
    } else if name == CALLABLE_THEOREM_HEAD {
        Some("callable theorem")
    } else {
        name.strip_prefix(PREDICATE_PREFIX)
            .or_else(|| name.strip_prefix(NAT_TERM_PREFIX))
    }
}

/// Recognize the arguments of an unqualified `Proof<...>` in a type annotation.
/// These arguments are checked as proposition syntax.
pub(crate) fn is_proposition_arguments(node: &SyntaxNode) -> bool {
    if node.kind() != SyntaxKind::GENERIC_ARG_LIST {
        return false;
    }
    let mut previous = std::iter::successors(node.prev_sibling_or_token(), |element| {
        element.prev_sibling_or_token()
    })
    .filter(|element| !element.kind().is_trivia());
    previous.next().is_some_and(|element| {
        element
            .into_token()
            .is_some_and(|token| token.kind() == SyntaxKind::IDENT && token.text() == PROOF_TYPE)
    }) && !previous
        .next()
        .is_some_and(|element| element.kind() == SyntaxKind::DOT)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProofPredicates {
    by_name: FxHashMap<String, Option<Predicate>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Predicate {
    head: TyCon,
    arity: usize,
}

impl ProofPredicates {
    fn add(&mut self, name: String, predicate: Predicate) {
        self.by_name
            .entry(name)
            .and_modify(|old| {
                if old.as_ref() != Some(&predicate) {
                    *old = None;
                }
            })
            .or_insert(Some(predicate));
    }

    fn import(&mut self, ty: &Ty, qualifiers: &[&str]) {
        if let Some((head, args)) = proposition_parts(ty) {
            let Some(name) = head.name.strip_prefix(PREDICATE_PREFIX) else {
                return;
            };
            let predicate = Predicate {
                head: head.clone(),
                arity: args.len(),
            };
            self.add(name.to_owned(), predicate.clone());
            for qualifier in qualifiers {
                self.add(format!("{qualifier}.{name}"), predicate.clone());
            }
            return;
        }
        for part in ty.parts() {
            self.import(part, qualifiers);
        }
    }

    fn resolve(&self, name: &str) -> Result<&Predicate, String> {
        match self.by_name.get(name) {
            Some(Some(predicate)) => Ok(predicate),
            Some(None) => Err(format!("proof predicate `{name}` is ambiguous")),
            None => Err(format!("unknown proof predicate `{name}`")),
        }
    }

    /// The proof type `Proof<args>` annotates, or why there is none: an
    /// error at `span` in the proposition, or in a Nat index of it.
    pub(crate) fn annotation(
        &self,
        mut args: Vec<Ty>,
        check_nat: impl Fn(Ty) -> Result<NatIndex, TypeError>,
        span: TextRange,
    ) -> Result<Ty, TypeError> {
        let proposition = |reason: String| TypeError::InvalidProposition { reason, span };
        if args.len() != 1 {
            return Err(proposition(
                "a `Proof` type requires exactly one proposition".into(),
            ));
        }
        let (head, args) = match args.remove(0) {
            Ty::App(head, args) => (*head, args),
            head => (head, vec![]),
        };
        let Ty::Con(con) = head else {
            return Err(proposition(
                "a `Proof` type requires a named proposition or `NatEq<lhs, rhs>`".into(),
            ));
        };
        if con.name == NAT_EQ {
            let [left, right] = args.try_into().map_err(|_: Vec<Ty>| {
                proposition("`NatEq` requires exactly two Nat arguments".into())
            })?;
            return Ok(nat_equality(
                annotation_nat_term(left, &check_nat, span)?,
                annotation_nat_term(right, &check_nat, span)?,
            ));
        }
        let predicate = self.resolve(&con.name).map_err(proposition)?;
        let indices = args
            .into_iter()
            .map(|arg| check_nat(arg).map(Ty::Nat))
            .collect::<Result<_, _>>()?;
        predicate.apply(indices).map_err(proposition)
    }
}

impl Predicate {
    fn apply(&self, indices: Vec<Ty>) -> Result<Ty, String> {
        if indices.len() != self.arity {
            return Err(format!(
                "proof predicate `{}` requires {} Nat arguments, found {}",
                self.head,
                self.arity,
                indices.len()
            ));
        }
        Ok(proof_type(self.head.clone(), indices))
    }
}

fn proof_type(head: TyCon, indices: Vec<Ty>) -> Ty {
    let head = Ty::Con(head);
    let proposition = if indices.is_empty() {
        head
    } else {
        Ty::App(Box::new(head), indices)
    };
    Ty::App(Box::new(Ty::Con(TyCon::new(PROOF_HEAD))), vec![proposition])
}

fn nat_equality(left: Ty, right: Ty) -> Ty {
    proof_type(TyCon::new(NAT_EQ_HEAD), vec![left, right])
}

fn proposition_parts(ty: &Ty) -> Option<(&TyCon, &[Ty])> {
    let [proposition] = ty.args_of(PROOF_HEAD)? else {
        return None;
    };
    let (head, args) = match proposition {
        Ty::App(head, args) => (head.as_ref(), args.as_slice()),
        head => (head, &[][..]),
    };
    match head {
        Ty::Con(head) if head.name == NAT_EQ_HEAD || head.name.starts_with(PREDICATE_PREFIX) => {
            Some((head, args))
        }
        _ => None,
    }
}

// Symbolic arithmetic stays inside NatEq. Its leaves use the same NatIndex
// representation as ordinary indices, and its heads cannot name source types.
#[derive(Clone, Copy)]
enum NatOperation {
    Succ,
    Add,
    Sub,
    Mul,
}

impl NatOperation {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "Succ" => Some(Self::Succ),
            "NatAdd" => Some(Self::Add),
            "NatSub" => Some(Self::Sub),
            "NatMul" => Some(Self::Mul),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Succ => "Succ",
            Self::Add => "NatAdd",
            Self::Sub => "NatSub",
            Self::Mul => "NatMul",
        }
    }

    fn apply(self, args: Vec<Ty>) -> Option<Ty> {
        fn literal(ty: &Ty) -> Option<u64> {
            match ty {
                Ty::Nat(NatIndex::Literal(value)) => Some(*value),
                _ => None,
            }
        }

        let folded = match (self, args.as_slice()) {
            (Self::Succ, [Ty::Nat(index)]) => Some(Ty::Nat(index.clone().successor())),
            (Self::Succ, [_]) => None,
            (Self::Add, [left, right]) => match (literal(left), literal(right)) {
                (Some(0), _) => Some(right.clone()),
                (_, Some(0)) => Some(left.clone()),
                (Some(1), _) => Self::Succ.apply(vec![right.clone()]),
                (_, Some(1)) => Self::Succ.apply(vec![left.clone()]),
                (Some(a), Some(b)) => a.checked_add(b).map(NatIndex::Literal).map(Ty::Nat),
                _ => None,
            },
            (Self::Sub, [left, right]) => match (literal(left), literal(right)) {
                (Some(0), _) | (_, Some(0)) => Some(left.clone()),
                (Some(a), Some(b)) => Some(Ty::Nat(NatIndex::Literal(a.saturating_sub(b)))),
                _ => None,
            },
            (Self::Mul, [left, right]) => match (literal(left), literal(right)) {
                (Some(0), _) | (_, Some(0)) => Some(Ty::Nat(NatIndex::Literal(0))),
                (Some(1), _) => Some(right.clone()),
                (_, Some(1)) => Some(left.clone()),
                (Some(a), Some(b)) => a.checked_mul(b).map(NatIndex::Literal).map(Ty::Nat),
                _ => None,
            },
            _ => return None,
        };
        Some(folded.unwrap_or_else(|| {
            let head = TyCon::new(format!("{NAT_TERM_PREFIX}{}", self.name()));
            Ty::App(Box::new(Ty::Con(head)), args)
        }))
    }
}

fn annotation_nat_term(
    term: Ty,
    check_nat: &impl Fn(Ty) -> Result<NatIndex, TypeError>,
    span: TextRange,
) -> Result<Ty, TypeError> {
    if let Ty::App(head, args) = term {
        if let Ty::Con(con) = head.as_ref() {
            if let Some(operation) = NatOperation::from_name(&con.name) {
                let args = args
                    .into_iter()
                    .map(|arg| annotation_nat_term(arg, check_nat, span))
                    .collect::<Result<_, _>>()?;
                return operation
                    .apply(args)
                    .ok_or_else(|| TypeError::InvalidIndex {
                        reason: format!("incorrect number of Nat arguments to `{}`", con.name),
                        span,
                    });
            }
        }
        check_nat(Ty::App(head, args)).map(Ty::Nat)
    } else {
        check_nat(term).map(Ty::Nat)
    }
}

pub(crate) struct ProofValue {
    pub name: String,
    pub ty: Ty,
    pub span: TextRange,
}

pub(crate) struct ProofValues {
    pub predicates: ProofPredicates,
    pub values: Vec<ProofValue>,
    pub errors: Vec<TypeError>,
}

pub(crate) fn collect(tree: &SourceFile, import_ctx: &ImportContext) -> ProofValues {
    let mut predicates = ProofPredicates::default();
    for (path, module) in &import_ctx.module_exports {
        let mut qualifiers = vec![path.as_str()];
        if module.module_name != *path {
            qualifiers.push(&module.module_name);
        }
        qualifiers.extend(
            import_ctx
                .module_qualifiers
                .iter()
                .filter(|(_, target)| *target == path || **target == module.module_name)
                .map(|(qualifier, _)| qualifier.as_str()),
        );
        for scheme in module.functions.values() {
            predicates.import(&scheme.ty, &qualifiers);
        }
    }

    let mut declarations = Vec::new();
    let mut errors = Vec::new();
    let mut sorts = LogicSorts::default();
    for block in tree.logic_blocks() {
        let Some(range) = block.body_range() else {
            continue;
        };
        let offset = block.syntax().text_range().start();
        let body = block
            .syntax()
            .text()
            .slice(TextRange::new(range.start() - offset, range.end() - offset))
            .to_string();
        let start = range.start();
        match logic::parse_body(&body) {
            Ok(file) => {
                for import in &file.imports {
                    sorts.shadow(import.alias.as_deref().unwrap_or(&import.name));
                }
                declarations.extend(file.declarations.into_iter().map(|decl| (start, decl)))
            }
            Err(mut error) => {
                // At its place in the file, as are the block's declarations.
                error.span = shift(start, error.span);
                if let Some((_, related)) = &mut error.related {
                    *related = shift(start, *related);
                }
                errors.push(TypeError::LogicSyntax { error });
            }
        }
    }

    let origin = import_ctx.current_module.as_deref().unwrap_or_default();
    let mut proof_names = FxHashSet::default();
    for (_, declaration) in &declarations {
        match declaration {
            Declaration::Proof { name, .. } => {
                proof_names.insert(name.as_str());
            }
            Declaration::Theorem { name, .. } | Declaration::Definition { name, .. } => {
                sorts.shadow(name);
            }
            Declaration::Inductive {
                name, constructors, ..
            } => {
                sorts.shadow(name);
                for constructor in constructors {
                    sorts.shadow(&constructor.name);
                }
            }
        }
    }

    // A native theorem schema resolves its propositions against declarations
    // in this module. Proof annotations can also use imported predicates.
    let mut local_predicates = ProofPredicates::default();
    for (_, declaration) in &declarations {
        if let Some((name, arity)) = nat_predicate(declaration, sorts) {
            let head = TyCon::indexed(&format!("{PREDICATE_PREFIX}{name}"), origin);
            let predicate = Predicate { head, arity };
            local_predicates.add(name.to_owned(), predicate.clone());
            if name == NAT_EQ && !origin.is_empty() {
                predicates.add(format!("{origin}.{name}"), predicate.clone());
            }
            predicates.add(name.to_owned(), predicate);
        }
    }

    let mut values = Vec::new();
    let mut exported = FxHashMap::default();
    for (start, declaration) in &declarations {
        let Declaration::Theorem {
            name,
            public: true,
            params,
            proposition,
            span,
        } = declaration
        else {
            continue;
        };
        let callable = if params.is_empty() {
            None
        } else {
            let Some(schema) = theorem_schema(params, proposition, &local_predicates, sorts) else {
                continue;
            };
            Some(schema)
        };
        let span = source_range(*start, *span);
        let result = if let Some(&first) = exported.get(name) {
            Err(TypeError::DuplicateProofValue {
                name: name.clone(),
                first,
                span,
            })
        } else if !proof_names.contains(name.as_str()) {
            exported.insert(name, span);
            Err(TypeError::MissingEmbeddedProof {
                theorem: name.clone(),
                span,
            })
        } else {
            exported.insert(name, span);
            callable
                .map_or_else(|| closed_proposition(proposition, &predicates, sorts), Ok)
                .map_err(|reason| TypeError::InvalidProposition { reason, span })
        };
        match result {
            Ok(ty) => values.push(ProofValue {
                name: name.clone(),
                ty,
                span,
            }),
            Err(error) => errors.push(error),
        }
    }
    ProofValues {
        predicates,
        values,
        errors,
    }
}

fn source_range(start: TextSize, span: Span) -> TextRange {
    TextRange::new(
        start + TextSize::from(span.start),
        start + TextSize::from(span.end),
    )
}

/// `span`, of a block body that starts at `start`, in the file.
fn shift(start: TextSize, span: Span) -> Span {
    let range = source_range(start, span);
    Span::new(range.start().into(), range.end().into())
}

fn named(term: &Term, expected: &str) -> bool {
    matches!(term, Term::Name(name) if matches!(name.as_slice(), [name] if name == expected))
}

#[derive(Clone, Copy, Default)]
struct LogicSorts {
    nat_shadowed: bool,
    prop_shadowed: bool,
}

impl LogicSorts {
    fn shadow(&mut self, name: &str) {
        match name {
            "Nat" => self.nat_shadowed = true,
            "Prop" => self.prop_shadowed = true,
            _ => {}
        }
    }

    fn is_nat(self, term: &Term) -> bool {
        !self.nat_shadowed && named(term, "Nat")
    }

    fn is_prop(self, term: &Term) -> bool {
        !self.prop_shadowed && named(term, "Prop")
    }
}

fn nat_predicate(declaration: &Declaration, mut sorts: LogicSorts) -> Option<(&str, usize)> {
    let (name, params, mut result): (&str, &[Binder], &Term) = match declaration {
        Declaration::Definition {
            name,
            params,
            result,
            ..
        } => (name, params, result),
        Declaration::Inductive {
            name,
            params,
            kind: Some(kind),
            ..
        } => (name, params, kind),
        _ => return None,
    };
    for param in params {
        if !sorts.is_nat(&param.ty) {
            return None;
        }
        sorts.shadow(&param.name);
    }
    let mut arity = params.len();
    if matches!(declaration, Declaration::Inductive { .. }) {
        while let Term::Binary(Binary::Arrow, input, output) = result {
            if !sorts.is_nat(input) {
                return None;
            }
            arity += 1;
            result = output;
        }
    }
    sorts.is_prop(result).then_some((name, arity))
}

fn named_proposition(term: &Term) -> Result<(&str, &[Term]), String> {
    let (head, args) = match term {
        Term::Call(head, args) => (head.as_ref(), args.as_slice()),
        head => (head, &[][..]),
    };
    let Term::Name(path) = head else {
        return Err("an exported theorem requires a named Nat proposition".into());
    };
    // Embedded definitions currently share a flat logic namespace.
    let [name] = path.as_slice() else {
        return Err("an exported theorem requires an unqualified logic predicate".into());
    };
    Ok((name, args))
}

fn closed_proposition(
    term: &Term,
    predicates: &ProofPredicates,
    sorts: LogicSorts,
) -> Result<Ty, String> {
    let bindings = FxHashMap::default();
    if let Term::Binary(Binary::Eq, left, right) = term {
        return logic_nat_equality(left, right, &bindings, sorts).ok_or_else(|| {
            "an exported equality theorem requires two supported Nat terms".to_owned()
        });
    }
    let (name, args) = named_proposition(term)?;
    let predicate = predicates.resolve(name)?;
    let indices = args
        .iter()
        .map(|arg| {
            logic_nat_term(arg, &bindings, sorts, false).ok_or(
                "an exported theorem's predicate arguments must use Nat literals or Nat.succ",
            )
        })
        .collect::<Result<_, _>>()?;
    predicate.apply(indices)
}

enum BinderKind {
    Nat,
    Evidence,
}

fn theorem_schema(
    params: &[Binder],
    proposition: &Term,
    local_predicates: &ProofPredicates,
    mut sorts: LogicSorts,
) -> Option<Ty> {
    let mut bindings = FxHashMap::default();
    let mut slots = Vec::with_capacity(params.len());
    for param in params {
        let (kind, slot) = if sorts.is_nat(&param.ty) {
            (
                BinderKind::Nat,
                Ty::Nat(NatIndex::Parameter(param.name.clone())),
            )
        } else {
            let proof = schema_proposition(&param.ty, local_predicates, &bindings, sorts)?;
            (BinderKind::Evidence, proof)
        };
        if bindings.insert(param.name.as_str(), kind).is_some() {
            return None;
        }
        sorts.shadow(&param.name);
        slots.push(slot);
    }
    let result = schema_proposition(proposition, local_predicates, &bindings, sorts)?;
    Some(Ty::App(
        Box::new(Ty::Con(TyCon::new(CALLABLE_THEOREM_HEAD))),
        vec![Ty::Tuple(slots), result],
    ))
}

fn schema_proposition(
    term: &Term,
    local_predicates: &ProofPredicates,
    bindings: &FxHashMap<&str, BinderKind>,
    sorts: LogicSorts,
) -> Option<Ty> {
    if let Term::Binary(Binary::Eq, left, right) = term {
        return logic_nat_equality(left, right, bindings, sorts);
    }
    let (name, args) = named_proposition(term).ok()?;
    if bindings.contains_key(name) {
        return None;
    }
    let predicate = local_predicates.resolve(name).ok()?;
    let indices = args
        .iter()
        .map(|arg| logic_nat_term(arg, bindings, sorts, false))
        .collect::<Option<Vec<_>>>()?;
    predicate.apply(indices).ok()
}

fn logic_nat_equality(
    left: &Term,
    right: &Term,
    bindings: &FxHashMap<&str, BinderKind>,
    sorts: LogicSorts,
) -> Option<Ty> {
    Some(nat_equality(
        logic_nat_term(left, bindings, sorts, true)?,
        logic_nat_term(right, bindings, sorts, true)?,
    ))
}

fn logic_nat_term(
    term: &Term,
    bindings: &FxHashMap<&str, BinderKind>,
    sorts: LogicSorts,
    allow_arithmetic: bool,
) -> Option<Ty> {
    match term {
        Term::Nat(value) => Some(Ty::Nat(NatIndex::Literal(*value))),
        Term::Name(path) => match path.as_slice() {
            [name] if matches!(bindings.get(name.as_str()), Some(BinderKind::Nat)) => {
                Some(Ty::Nat(NatIndex::Parameter(name.clone())))
            }
            _ => None,
        },
        Term::Call(head, args) if !sorts.nat_shadowed => {
            let (Term::Name(path), [argument]) = (head.as_ref(), args.as_slice()) else {
                return None;
            };
            (path.as_slice() == ["Nat", "succ"]).then_some(())?;
            let argument = logic_nat_term(argument, bindings, sorts, allow_arithmetic)?;
            NatOperation::Succ.apply(vec![argument])
        }
        Term::Binary(operator, left, right) if allow_arithmetic => {
            let operation = match operator {
                Binary::Add => NatOperation::Add,
                Binary::Sub => NatOperation::Sub,
                Binary::Mul => NatOperation::Mul,
                _ => return None,
            };
            let left = logic_nat_term(left, bindings, sorts, allow_arithmetic)?;
            let right = logic_nat_term(right, bindings, sorts, allow_arithmetic)?;
            operation.apply(vec![left, right])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check, check_with_imports, collect_exports, ModuleExports};

    fn parse(source: &str) -> mesh_parser::Parse {
        let parse = mesh_parser::parse(source);
        assert!(parse.errors().is_empty(), "{:?}", parse.errors());
        parse
    }

    #[test]
    fn exports_direct_nat_equality_for_ordinary_mesh_calls() {
        let parse = parse(
            "logic do
  pub theorem add_zero(n :: Nat) do n + 0 == n end
  proof add_zero do refl end
  pub theorem forward(n :: Nat, h :: Nat.succ(n) == n + 1) do Nat.succ(n) == n + 1 end
  proof forward do exact h end
  pub theorem two() do Nat.succ(1) == 2 end
  proof two do refl end
end
fn use_equality() -> Proof<NatEq<2, 2>> do
  forward(1, two)
end
fn use_add_zero() -> Proof<NatEq<4, 4>> do
  add_zero(4)
end",
        );
        let checked = check(&parse);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let exports = collect_exports(&parse, &checked);
        let module = ModuleExports::new("Equalities".into(), &exports);
        assert_eq!(module.functions["two"].ty.to_string(), "Proof<NatEq<2, 2>>");
        let forward = module.functions["forward"].ty.callable_theorem().unwrap();
        let [CallableTheoremParam::Nat("n"), CallableTheoremParam::Evidence(premise)] =
            forward.params().collect::<Vec<_>>()[..]
        else {
            panic!("incorrect theorem slot order");
        };
        assert_eq!(premise, forward.result);
        assert_eq!(premise.to_string(), "Proof<NatEq<Succ<n>, Succ<n>>>");
        let (head, _) = proposition_parts(premise).unwrap();
        assert_ne!(head, &TyCon::new("NatEq"));
        assert_eq!(head.indexed_origin, None);
    }

    #[test]
    fn preserves_symbolic_equality_terms_and_folds_them_after_substitution() {
        let source = parse(
            "logic do
  pub theorem arithmetic(a :: Nat, b :: Nat) do
    Nat.succ((a + b) * (a - b)) == Nat.succ((a + b) * (a - b))
  end
  proof arithmetic do refl end
end
fn symbolic<x :: Nat, y :: Nat>() -> Proof<NatEq<Succ<NatMul<NatAdd<x, y>, NatSub<x, y>>>, Succ<NatMul<NatAdd<x, y>, NatSub<x, y>>>>> do
  arithmetic(x, y)
end
fn concrete() -> Proof<NatEq<6, 6>> do
  arithmetic(3, 2)
end
fn zero() -> Proof<NatEq<1, 1>> do
  arithmetic(2, 3)
end",
        );
        let checked = check(&source);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let theorem = checked.proof_values["arithmetic"]
            .callable_theorem()
            .unwrap();
        assert_eq!(
            theorem.result.to_string(),
            "Proof<NatEq<Succ<NatMul<NatAdd<a, b>, NatSub<a, b>>>, Succ<NatMul<NatAdd<a, b>, NatSub<a, b>>>>>",
        );
    }

    #[test]
    fn a_same_named_predicate_keeps_its_qualified_identity() {
        let source = parse(
            "logic do
  fn NatEq(a :: Nat, b :: Nat) -> Prop do a == b end
  pub theorem named() do NatEq(2, 2) end
  proof named do refl end
  pub theorem direct() do 2 == 2 end
  proof direct do refl end
end
fn use_named() -> Proof<Equalities.NatEq<2, 2>> do named end
fn use_direct() -> Proof<NatEq<2, 2>> do direct end",
        );
        let checked = check_with_imports(
            &source,
            &ImportContext {
                current_module: Some("Equalities".into()),
                ..ImportContext::empty()
            },
        );
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        assert_ne!(
            checked.proof_values["named"],
            checked.proof_values["direct"]
        );
    }

    #[test]
    fn rejects_untyped_or_shadowed_equality_operands_and_missing_proofs() {
        let source = parse(
            "logic do
  fn NatAdd(a :: Nat, b :: Nat) -> Nat do a end
  pub theorem custom(n :: Nat) do NatAdd(n, 0) == n end
  proof custom do refl end
  pub theorem shadowed(Nat :: Nat) do Nat.succ(Nat) == Nat.succ(Nat) end
  proof shadowed do refl end
  pub theorem unchecked(n :: Nat) do n == n end
  pub theorem boolean_operand(n :: Nat) do 0 * true == n end
  proof boolean_operand do refl end
end",
        );
        let proofs = collect(&source.tree(), &ImportContext::empty());
        assert!(proofs.values.is_empty());
        assert!(
            matches!(&proofs.errors[..], [TypeError::MissingEmbeddedProof { theorem, .. }] if theorem == "unchecked"),
            "{:?}",
            proofs.errors
        );
    }

    #[test]
    fn equality_evidence_cannot_satisfy_a_different_nat_premise() {
        let source = parse(
            "logic do
  pub theorem add_zero(n :: Nat) do n + 0 == n end
  proof add_zero do refl end
  pub theorem reflect(n :: Nat, h :: n == 2) do 2 == n end
  proof reflect do exact Eq.symm(h) end
end
fn use_wrong_evidence() do
  reflect(2, add_zero(1))
end",
        );
        let checked = check(&source);
        assert!(
            matches!(&checked.errors[..], [TypeError::Mismatch { expected, found, .. }]
                if expected.to_string() == "2" && found.to_string() == "1"),
            "{:?}",
            checked.errors,
        );

        let malformed = parse(
            "fn bad<x>(h :: Proof<NatEq<NatMul<0, x>, 0>>) do
  h
end",
        );
        let checked = check(&malformed);
        assert!(
            checked.errors.iter().any(|error| matches!(
                error,
                TypeError::InvalidIndex { reason, .. }
                    if reason.contains("expected a Nat index")
            )),
            "{:?}",
            checked.errors,
        );
    }

    #[test]
    fn exports_a_theorem_with_mixed_slots_and_preserves_predicate_origin() {
        let parse = parse(
            "logic do
  fn Fits(n :: Nat) -> Prop do n == n end
  inductive Allowed -> Nat -> Nat -> Prop do
    Permit(n :: Nat, m :: Nat) -> Allowed(n, m)
  end
  pub theorem twice(n :: Nat, first :: Fits(n), Nat :: Nat, second :: Fits(2)) do Allowed(n, Nat) end
  proof twice do exact Allowed.Permit(n, Nat) end
end",
        );
        let checked = check(&parse);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let exports = collect_exports(&parse, &checked);
        let module = ModuleExports::new("Permits".into(), &exports);
        let schema = &module.functions["twice"].ty;
        let theorem = schema.callable_theorem().unwrap();
        let slots = theorem.params().collect::<Vec<_>>();
        let [CallableTheoremParam::Nat("n"), CallableTheoremParam::Evidence(first), CallableTheoremParam::Nat("Nat"), CallableTheoremParam::Evidence(second)] =
            slots.as_slice()
        else {
            panic!("incorrect theorem slot order: {schema}");
        };

        let mut indices = FxHashMap::default();
        assert!(instantiate_theorem_proof(theorem.result, &indices).is_none());
        indices.insert("n".into(), NatIndex::Literal(1));
        let first = instantiate_theorem_proof(first, &indices).unwrap();
        let second = instantiate_theorem_proof(second, &indices).unwrap();
        assert_eq!(first.to_string(), "Proof<Fits<1>>");
        assert_eq!(second.to_string(), "Proof<Fits<2>>");

        indices.insert("Nat".into(), NatIndex::Parameter("caller_index".into()));
        let result = instantiate_theorem_proof(theorem.result, &indices).unwrap();
        assert_eq!(result.to_string(), "Proof<Allowed<1, caller_index>>");
        for proof in [first, second, result] {
            let (head, _) = proposition_parts(&proof).unwrap();
            assert_eq!(
                head.indexed_origin.as_deref().map(String::as_str),
                Some("Permits"),
            );
        }

        let mut predicates = ProofPredicates::default();
        predicates.import(schema, &["Permits"]);
        assert_eq!(predicates.resolve("Permits.Fits").unwrap().arity, 1);
        assert_eq!(predicates.resolve("Permits.Allowed").unwrap().arity, 2);
    }

    #[test]
    fn leaves_other_parameterized_statements_in_logic() {
        let imported = parse(
            "logic do
  fn Foreign(n :: Nat) -> Prop do n == n end
  pub theorem foreign() do Foreign(2) end
  proof foreign do exact Eq.refl(2) end
end",
        );
        let checked = check(&imported);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let exports = collect_exports(&imported, &checked);
        let mut context = ImportContext::empty();
        context
            .module_exports
            .insert("Other".into(), ModuleExports::new("Other".into(), &exports));

        let source = parse(
            "logic do
  fn Fits(n :: Nat) -> Prop do n == n end
  pub theorem generic(n :: Nat, equal :: true == true) do Fits(n) end
  proof generic do exact equal end
  pub theorem arithmetic(n :: Nat) do Fits(n + 1) end
  proof arithmetic do exact Eq.refl(n + 1) end
  pub theorem foreign_result(n :: Nat) do Foreign(n) end
  proof foreign_result do exact Eq.refl(n) end
  pub theorem result_is_shadowed(n :: Nat, Fits :: Prop, h :: Fits) do Fits end
  proof result_is_shadowed do exact h end
end",
        );
        let proofs = collect(&source.tree(), &context);
        assert!(proofs.errors.is_empty(), "{:?}", proofs.errors);
        assert!(proofs.values.is_empty());
    }

    #[test]
    fn refuses_a_shadowed_nat_sort_and_requires_a_proof_for_a_schema() {
        let source = "logic do
  fn Ready() -> Prop do 2 == 2 end
  pub theorem available(n :: Nat) do Ready end
end";
        let proofs = collect(&parse(source).tree(), &ImportContext::empty());
        assert!(proofs.values.is_empty());
        assert!(
            matches!(&proofs.errors[..], [TypeError::MissingEmbeddedProof { theorem, .. }] if theorem == "available"),
            "{:?}",
            proofs.errors
        );

        let shadowed = source.replace(
            "  fn Ready",
            "  inductive Nat do\n    Token\n  end\n  fn Ready",
        );
        let proofs = collect(&parse(&shadowed).tree(), &ImportContext::empty());
        assert!(proofs.errors.is_empty(), "{:?}", proofs.errors);
        assert!(proofs.values.is_empty());
    }

    fn range_of(source: &str, needle: &str) -> TextRange {
        let start = source.find(needle).unwrap() as u32;
        TextRange::new(start.into(), (start + needle.len() as u32).into())
    }

    #[test]
    fn a_logic_syntax_error_is_at_its_place_in_the_file() {
        let source = "fn before() = 1\nlogic do\n  theorem two() do 2 = 2 end\nend\n";
        let proofs = collect(&parse(source).tree(), &ImportContext::empty());
        let [TypeError::LogicSyntax { error }] = &proofs.errors[..] else {
            panic!("{:?}", proofs.errors);
        };
        assert_eq!(error.code, mesh_diagnostics::Code::LogicSyntax);
        assert_eq!(error.message, "expected `end`, found `=`");
        let equals = source.find(" = 2").unwrap() as u32 + 1;
        assert_eq!(error.span, Span::new(equals, equals + 1));
        let opened = source.find("do 2").unwrap() as u32;
        assert_eq!(
            error.related,
            Some(("block opened here".into(), Span::new(opened, opened + 2)))
        );
    }

    #[test]
    fn a_second_export_of_a_name_points_at_the_first() {
        let source = "logic do
  pub theorem two() do 2 == 2 end
  proof two do refl end
  pub theorem two() do 1 + 1 == 2 end
end";
        let proofs = collect(&parse(source).tree(), &ImportContext::empty());
        let [TypeError::DuplicateProofValue { name, first, span }] = &proofs.errors[..] else {
            panic!("{:?}", proofs.errors);
        };
        assert_eq!(name, "two");
        assert_eq!(*first, range_of(source, "pub theorem two() do 2 == 2 end"));
        assert_eq!(
            *span,
            range_of(source, "pub theorem two() do 1 + 1 == 2 end")
        );
    }

    #[test]
    fn an_exported_statement_without_a_proof_type_is_an_invalid_proposition() {
        let source = "logic do
  pub theorem yes() do true end
  proof yes do refl end
end";
        let proofs = collect(&parse(source).tree(), &ImportContext::empty());
        let [TypeError::InvalidProposition { reason, span }] = &proofs.errors[..] else {
            panic!("{:?}", proofs.errors);
        };
        assert_eq!(
            reason,
            "an exported theorem requires a named Nat proposition"
        );
        assert_eq!(*span, range_of(source, "pub theorem yes() do true end"));
    }
}
