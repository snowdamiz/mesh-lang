use mesh_parser::ast::item::FnDef;
use mesh_parser::ast::item::IndexParam;
use mesh_parser::{AstNode, SyntaxElement, SyntaxNode};

use super::{
    collect_annotation_tokens, generic_param_names, significant_range, FxHashMap, InferCtx,
    NatIndex, SumTypeDefInfo, SyntaxKind, TextRange, Ty, TyCon, TyVar, TypeError, TypeRegistry,
};

/// The named family's identity and its Nat parameter positions.
#[derive(Clone, Debug)]
pub struct IndexedSumTypeInfo {
    pub origin: String,
    pub index_positions: Vec<usize>,
}

/// The variant's local Nat parameters and exact result, scoped over those
/// parameters and the family's ordinary type parameters.
#[derive(Clone, Debug)]
pub struct IndexedVariantInfo {
    pub index_params: Vec<String>,
    pub result: Ty,
}

impl SumTypeDefInfo {
    pub fn runtime_generic_params(&self) -> impl Iterator<Item = &String> {
        self.generic_params
            .iter()
            .enumerate()
            .filter_map(|(pos, name)| (!self.is_index_position(pos)).then_some(name))
    }

    fn is_index_position(&self, pos: usize) -> bool {
        self.indexed
            .as_ref()
            .is_some_and(|info| info.index_positions.contains(&pos))
    }

    /// The source type constructor, including the checked origin of an indexed family.
    pub fn head(&self) -> TyCon {
        match &self.indexed {
            Some(indexed) => TyCon::indexed(&self.name, &indexed.origin),
            None => TyCon::new(&self.name),
        }
    }
}

impl TypeRegistry {
    pub(super) fn register_imported_sum_type(
        &mut self,
        info: &SumTypeDefInfo,
        module: &str,
        paths: impl IntoIterator<Item = String>,
    ) {
        if let Some(indexed) = &info.indexed {
            let identity = (indexed.origin.clone(), info.name.clone());
            for path in paths
                .into_iter()
                .chain([module.to_owned(), indexed.origin.clone()])
            {
                self.indexed_type_qualifiers
                    .insert(format!("{path}.{}", info.name), identity.clone());
            }
        }
        self.register_sum_type(info.clone());
    }

    pub(super) fn indexed_family(&self, con: &TyCon) -> Option<&SumTypeDefInfo> {
        let (origin, name) = if let Some(origin) = &con.indexed_origin {
            (Some(origin.as_str()), con.name.as_str())
        } else if con.name.contains('.') {
            let (origin, name) = self.indexed_type_qualifiers.get(&con.name)?;
            (Some(origin.as_str()), name.as_str())
        } else {
            (None, con.name.as_str())
        };
        let family = self.lookup_sum_type(name)?;
        let index = family.indexed.as_ref()?;
        origin
            .is_none_or(|origin| origin == index.origin)
            .then_some(family)
    }

    pub(super) fn check_indexed_layout_collision(
        &self,
        ctx: &mut InferCtx,
        name: &str,
        origin: Option<&str>,
        span: TextRange,
    ) {
        let conflicts_with_indexed = self
            .lookup_sum_type(name)
            .and_then(|def| def.indexed.as_ref())
            .is_some_and(|def| Some(def.origin.as_str()) != origin);
        let conflicts_with_ordinary = origin.is_some()
            && (self.struct_defs.contains_key(name)
                || self
                    .sum_type_defs
                    .get(name)
                    .is_some_and(|def| def.indexed.is_none())
                || ctx.builtin_types.contains(name));
        if conflicts_with_indexed || conflicts_with_ordinary {
            invalid_index(
                ctx,
                span,
                format!("indexed type `{name}` shares a runtime type name with another definition"),
            );
        }
    }
}

pub(super) fn invalid_index(ctx: &mut InferCtx, span: TextRange, reason: String) -> TypeError {
    let error = TypeError::InvalidIndex { reason, span };
    ctx.errors.push(error.clone());
    error
}

pub(super) fn index_param_name(parameter: IndexParam) -> Option<String> {
    parameter.name()?.text()
}

pub(super) fn index_param_names(node: &SyntaxNode) -> impl Iterator<Item = String> + '_ {
    generic_entries(node)
        .filter_map(|element| index_param_name(IndexParam::cast(element.into_node()?)?))
}

pub(super) fn generic_entries(node: &SyntaxNode) -> impl Iterator<Item = SyntaxElement> + '_ {
    node.children()
        .filter(|child| child.kind() == SyntaxKind::GENERIC_PARAM_LIST)
        .flat_map(|list| list.children_with_tokens())
}

pub(super) fn index_family_info(node: &SyntaxNode, origin: &str) -> Option<IndexedSumTypeInfo> {
    let index_positions: Vec<_> = generic_entries(node)
        .filter(|entry| matches!(entry.kind(), SyntaxKind::IDENT | SyntaxKind::INDEX_PARAM))
        .enumerate()
        .filter_map(|(i, entry)| (entry.kind() == SyntaxKind::INDEX_PARAM).then_some(i))
        .collect();
    (!index_positions.is_empty()).then(|| IndexedSumTypeInfo {
        origin: origin.to_owned(),
        index_positions,
    })
}

pub(super) fn check_index_syntax(ctx: &mut InferCtx, node: &SyntaxNode) {
    for child in node.descendants() {
        match child.kind() {
            SyntaxKind::INDEX_PARAM => {
                let param = IndexParam::cast(child).expect("index parameter");
                let span = significant_range(param.syntax());
                let mut tokens = Vec::new();
                if let Some(kind) = param.kind_annotation() {
                    collect_annotation_tokens(kind.syntax(), &mut tokens);
                }
                if !matches!(tokens.as_slice(), [(SyntaxKind::IDENT, kind)] if kind == "Nat") {
                    invalid_index(ctx, span, "an index parameter must have kind `Nat`".into());
                }
                let owner = param.syntax().parent().and_then(|list| list.parent());
                if !owner.is_some_and(|owner| match owner.kind() {
                    SyntaxKind::FN_DEF => !owner
                        .ancestors()
                        .any(|ancestor| ancestor.kind() == SyntaxKind::IMPL_DEF),
                    SyntaxKind::SUM_TYPE_DEF | SyntaxKind::VARIANT_DEF => true,
                    _ => false,
                }) {
                    invalid_index(
                        ctx,
                        span,
                        "index parameters are supported on sum types, variants and functions"
                            .into(),
                    );
                }
            }
            SyntaxKind::NAT_INDEX => {
                let token = child
                    .children_with_tokens()
                    .filter_map(SyntaxElement::into_token)
                    .find(|token| token.kind() == SyntaxKind::INT_LITERAL);
                if let Some(token) = token {
                    if parse_nat_literal(token.text()).is_none() {
                        invalid_index(
                            ctx,
                            token.text_range(),
                            "a Nat literal must fit in an unsigned 64-bit integer".into(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

pub(super) fn parse_nat_literal(text: &str) -> Option<u64> {
    let digits = text.replace('_', "");
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(digits) = digits.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix).ok();
        }
    }
    digits.parse().ok()
}

pub(super) fn function_type_params(ctx: &mut InferCtx, node: &SyntaxNode) -> FxHashMap<String, Ty> {
    let indices: Vec<String> = index_param_names(node).collect();
    let mut params = FxHashMap::default();
    for name in generic_param_names(node) {
        let ty = if indices.contains(&name) {
            ctx.fresh_index(Some(&name))
        } else {
            ctx.fresh_var()
        };
        if params.insert(name.clone(), ty).is_some() && !indices.is_empty() {
            invalid_index(
                ctx,
                significant_range(node),
                format!("duplicate parameter `{name}`"),
            );
        }
    }
    params
}

pub(super) fn check_index_bounds(
    ctx: &mut InferCtx,
    fun: &FnDef,
    params: &FxHashMap<String, Ty>,
    bounds: &[(String, String)],
) {
    for (name, _) in bounds {
        if matches!(params.get(name), Some(Ty::Nat(_))) {
            invalid_index(
                ctx,
                significant_range(fun.syntax()),
                format!("Nat parameter `{name}` cannot have a trait bound"),
            );
        }
    }
}

pub(super) fn contains_indexed_type(ty: &Ty) -> bool {
    matches!(ty, Ty::Con(con) if con.indexed_origin.is_some())
        || ty.parts().any(contains_indexed_type)
}

/// Turn raw annotation syntax into kind-checked types and Nat terms. A Nat
/// parameter belongs to the nearest declaration that binds its name.
pub(super) fn check_indexed_type(
    ty: Ty,
    node: &SyntaxNode,
    registry: &TypeRegistry,
) -> Result<Ty, String> {
    let lookup = |name: &str| {
        for enclosing in node.ancestors() {
            if index_param_names(&enclosing).any(|param| param == name) {
                return Some(true);
            }
            if generic_param_names(&enclosing)
                .iter()
                .any(|param| param == name)
            {
                return Some(false);
            }
        }
        None
    };
    check_type_kind(ty, registry, &lookup)
}

fn check_type_kind(
    ty: Ty,
    registry: &TypeRegistry,
    parameter_kind: &dyn Fn(&str) -> Option<bool>,
) -> Result<Ty, String> {
    use crate::proof_values::PROOF_TYPE;

    if ty.is_proof() {
        return Ok(ty);
    }
    match ty {
        Ty::Nat(index) => Err(format!("expected a type, found Nat index `{index}`")),
        Ty::Con(con) if con.name == PROOF_TYPE => {
            Err("a `Proof` type requires exactly one named proposition".into())
        }
        Ty::Con(con) => {
            match parameter_kind(&con.name) {
                Some(true) => {
                    return Err(format!(
                        "expected a type, found Nat parameter `{}`",
                        con.name
                    ))
                }
                Some(false) => return Ok(Ty::Con(con)),
                None => {}
            }
            if let Some(family) = registry.indexed_family(&con) {
                return Err(format!(
                    "indexed type `{}` requires {} explicit arguments",
                    family.name,
                    family.generic_params.len()
                ));
            }
            if is_unbound_succ(&con, registry) {
                return Err("a `Succ` index is allowed only in a Nat position".into());
            }
            reject_unresolved_indexed_qualifier(&con, registry)?;
            Ok(Ty::Con(con))
        }
        Ty::App(head, args) => {
            if matches!(head.as_ref(), Ty::Con(con) if con.name == PROOF_TYPE) {
                return registry
                    .proof_predicates
                    .annotation(args, |arg| check_nat_kind(arg, parameter_kind));
            }
            let family = head.con().and_then(|con| registry.indexed_family(con));
            if let Some(family) = family {
                if args.len() != family.generic_params.len() {
                    return Err(format!(
                        "indexed type `{}` requires {} arguments, found {}",
                        family.name,
                        family.generic_params.len(),
                        args.len()
                    ));
                }
                let args = args
                    .into_iter()
                    .enumerate()
                    .map(|(pos, arg)| {
                        if family.is_index_position(pos) {
                            check_nat_kind(arg, parameter_kind).map(Ty::Nat)
                        } else {
                            check_type_kind(arg, registry, parameter_kind)
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(Ty::App(Box::new(Ty::Con(family.head())), args));
            }
            if let Some(con) = head.con() {
                if con.indexed_origin.is_some() {
                    // Types from an exported signature can name a family in
                    // a transitive dependency that this module cannot name.
                    return Ok(Ty::App(head, args));
                }
                if is_unbound_succ(con, registry) && parameter_kind(&con.name).is_none() {
                    return Err("a `Succ` index is allowed only in a Nat position".into());
                }
                reject_unresolved_indexed_qualifier(con, registry)?;
            }
            Ok(Ty::App(
                Box::new(check_type_kind(*head, registry, parameter_kind)?),
                args.into_iter()
                    .map(|arg| check_type_kind(arg, registry, parameter_kind))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        Ty::Fun(params, result) => Ok(Ty::fun(
            params
                .into_iter()
                .map(|arg| check_type_kind(arg, registry, parameter_kind))
                .collect::<Result<Vec<_>, _>>()?,
            check_type_kind(*result, registry, parameter_kind)?,
        )),
        Ty::Tuple(items) => Ok(Ty::Tuple(
            items
                .into_iter()
                .map(|arg| check_type_kind(arg, registry, parameter_kind))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        other => Ok(other),
    }
}

fn is_unbound_succ(con: &TyCon, registry: &TypeRegistry) -> bool {
    con.name == "Succ"
        && registry.lookup_sum_type("Succ").is_none()
        && registry.lookup_struct("Succ").is_none()
        && registry.lookup_alias("Succ").is_none()
}

fn reject_unresolved_indexed_qualifier(con: &TyCon, registry: &TypeRegistry) -> Result<(), String> {
    if let Some((_, short)) = con.name.rsplit_once('.') {
        if registry
            .lookup_sum_type(short)
            .is_some_and(|def| def.indexed.is_some())
            && registry.lookup_alias(&con.name).is_none()
        {
            return Err(format!("unknown indexed type `{}`", con.name));
        }
    }
    Ok(())
}

fn check_nat_kind(
    ty: Ty,
    parameter_kind: &dyn Fn(&str) -> Option<bool>,
) -> Result<NatIndex, String> {
    match ty {
        Ty::Nat(index) => Ok(index),
        Ty::Con(con) if parameter_kind(&con.name) == Some(true) => {
            Ok(NatIndex::Parameter(con.name))
        }
        Ty::App(head, mut args) if head.con_name() == Some("Succ") && args.len() == 1 => {
            Ok(check_nat_kind(args.remove(0), parameter_kind)?.successor())
        }
        ty => Err(format!("expected a Nat index, found `{ty}`")),
    }
}

pub(super) fn check_variant_result(
    def: &SumTypeDefInfo,
    result: &Ty,
    index_params: &[String],
) -> Result<(), String> {
    let Ty::App(head, args) = result else {
        return Err(format!(
            "an indexed constructor must return its family `{}`",
            def.name
        ));
    };
    if head.con() != Some(&def.head()) {
        return Err(format!(
            "an indexed constructor must return its family `{}`",
            def.name
        ));
    }
    let mut recoverable = Vec::new();
    for (pos, (name, arg)) in def.generic_params.iter().zip(args).enumerate() {
        if def.is_index_position(pos) {
            if let Ty::Nat(index) = arg {
                if let NatIndex::Parameter(param) = index.leaf() {
                    if !index_params.contains(param) {
                        return Err(format!("index `{param}` must be bound on the constructor"));
                    }
                    recoverable.push(param);
                }
            }
        } else if *arg != Ty::Con(TyCon::new(name)) {
            return Err(format!(
                "the constructor's result must preserve the type parameter `{name}`"
            ));
        }
    }
    if let Some(hidden) = index_params
        .iter()
        .find(|param| !recoverable.contains(param))
    {
        return Err(format!(
            "constructor index `{hidden}` must appear in the result"
        ));
    }
    Ok(())
}

pub(super) fn check_variant_field(ty: &Ty, index_params: &[String]) -> Result<(), String> {
    if let Ty::Nat(index) = ty {
        if let NatIndex::Parameter(name) = index.leaf() {
            if !index_params.contains(name) {
                return Err(format!("index `{name}` must be bound on the constructor"));
            }
        }
    }
    ty.parts()
        .try_for_each(|part| check_variant_field(part, index_params))
}

#[derive(PartialEq, Eq)]
enum IndexMatch {
    Exact,
    Unknown,
    Impossible,
}

impl IndexMatch {
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Impossible, _) | (_, Self::Impossible) => Self::Impossible,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            _ => Self::Exact,
        }
    }
}

pub(super) fn indexed_variant_can_match(variant: &IndexedVariantInfo, scrutinee: &Ty) -> bool {
    // Coverage asks whether the constructor is possible for any assignment
    // to the remaining indices in the scrutinee.
    match_indexed_variant(variant, scrutinee, |_| true).0 != IndexMatch::Impossible
}

type IndexEquations = FxHashMap<NatIndex, NatIndex>;

/// Solve the equalities between a constructor result template and a checked
/// scrutinee type. Template parameters and scrutinee variables have distinct
/// identities. These substitutions describe one arm, never the whole case.
fn match_indexed_variant(
    variant: &IndexedVariantInfo,
    scrutinee: &Ty,
    can_refine: impl Fn(TyVar) -> bool,
) -> (IndexMatch, IndexEquations) {
    let mut equations = IndexEquations::default();
    let (Ty::App(head, actual), Ty::App(wanted_head, wanted)) = (scrutinee, &variant.result) else {
        return (IndexMatch::Unknown, equations);
    };
    if head != wanted_head || actual.len() != wanted.len() {
        return (IndexMatch::Impossible, equations);
    }
    let result = wanted
        .iter()
        .zip(actual)
        .filter_map(|(wanted, actual)| match (wanted, actual) {
            (Ty::Nat(wanted), Ty::Nat(actual)) => Some(match_index(
                resolve_match_index(wanted, &equations),
                resolve_match_index(actual, &equations),
                &mut equations,
                &can_refine,
            )),
            (Ty::Nat(_), _) => Some(IndexMatch::Unknown),
            _ => None,
        })
        .fold(IndexMatch::Exact, IndexMatch::and);
    (result, equations)
}

fn resolve_match_index(index: &NatIndex, equations: &IndexEquations) -> NatIndex {
    index.map_leaf(|leaf| match equations.get(leaf) {
        Some(value) => resolve_match_index(value, equations),
        None => leaf.clone(),
    })
}

/// Compare two already resolved terms, preferring to bind a constructor
/// parameter to the scrutinee over introducing a new local equality.
fn match_index(
    mut wanted: NatIndex,
    mut actual: NatIndex,
    equations: &mut IndexEquations,
    can_refine: &impl Fn(TyVar) -> bool,
) -> IndexMatch {
    loop {
        if wanted == actual {
            return IndexMatch::Exact;
        }
        if &wanted == actual.leaf() || &actual == wanted.leaf() {
            // The same leaf with different offsets cannot be equal.
            return IndexMatch::Impossible;
        }
        let (var, value) = match (&wanted, &actual) {
            (NatIndex::Parameter(_), _) => (wanted, actual),
            (_, NatIndex::Parameter(_)) => (actual, wanted),
            (NatIndex::Variable(var), _) if can_refine(*var) => (wanted, actual),
            (_, NatIndex::Variable(var)) if can_refine(*var) => (actual, wanted),
            (NatIndex::Literal(_), NatIndex::Literal(_))
            | (NatIndex::Literal(0), NatIndex::Succ(_))
            | (NatIndex::Succ(_), NatIndex::Literal(0)) => return IndexMatch::Impossible,
            _ => match (wanted.predecessor(), actual.predecessor()) {
                (Some(a), Some(b)) => {
                    (wanted, actual) = (a, b);
                    continue;
                }
                _ => return IndexMatch::Unknown,
            },
        };
        equations.insert(var, value);
        return IndexMatch::Exact;
    }
}

pub(super) fn indexed_pattern_fields(
    ctx: &mut InferCtx,
    registry: &TypeRegistry,
    variant_name: &str,
    result: &Ty,
    scrutinee: &Ty,
    span: TextRange,
) -> Result<Option<Vec<Ty>>, TypeError> {
    let Some(head) = result.con().filter(|con| con.indexed_origin.is_some()) else {
        return Ok(None);
    };
    let scrutinee = ctx.resolve(scrutinee.clone());
    let Some(def) = registry.indexed_family(head) else {
        return Err(invalid_index(
            ctx,
            span,
            "import the indexed family before matching its constructors".into(),
        ));
    };
    let Some(variant) = def
        .variants
        .iter()
        .find(|variant| variant.name == variant_name)
    else {
        return Err(invalid_index(
            ctx,
            span,
            format!("unknown indexed constructor `{variant_name}`"),
        ));
    };
    let Some(indexed) = &variant.indexed else {
        return Err(invalid_index(
            ctx,
            span,
            format!("constructor `{variant_name}` has no checked index result"),
        ));
    };
    let (comparison, equations) =
        match_indexed_variant(indexed, &scrutinee, |var| ctx.is_rigid_index(var));
    if comparison != IndexMatch::Exact {
        let reason = match comparison {
            IndexMatch::Impossible => format!(
                "constructor `{variant_name}` cannot match `{}`",
                scrutinee.with_holes()
            ),
            _ => format!("matching `{variant_name}` requires a scrutinee with checked Nat indices"),
        };
        return Err(invalid_index(ctx, span, reason));
    }

    let witnesses: FxHashMap<_, _> = indexed
        .index_params
        .iter()
        .filter(|name| !equations.contains_key(&NatIndex::Parameter((*name).clone())))
        .map(|name| (name.clone(), ctx.fresh_arm_index(name)))
        .collect();
    let instantiate = |index: &NatIndex| {
        resolve_match_index(index, &equations).map_leaf(|leaf| match leaf {
            NatIndex::Parameter(name) => witnesses[name].clone(),
            _ => leaf.clone(),
        })
    };
    for (var, value) in &equations {
        if let NatIndex::Variable(var) = var {
            ctx.add_index_equation(*var, instantiate(value));
        }
    }
    let mut bindings: FxHashMap<_, _> = indexed
        .index_params
        .iter()
        .map(|name| {
            (
                name.clone(),
                Ty::Nat(instantiate(&NatIndex::Parameter(name.clone()))),
            )
        })
        .collect();
    if let Ty::App(_, args) = scrutinee {
        for (pos, name) in def.generic_params.iter().enumerate() {
            if !def.is_index_position(pos) {
                bindings.insert(name.clone(), args[pos].clone());
            }
        }
    }
    Ok(Some(
        variant
            .fields
            .iter()
            .map(|field| {
                use super::VariantFieldInfo::{Named, Positional};
                let ty = match field {
                    Positional(ty) | Named(_, ty) => ty,
                };
                super::with_declared_type_params(ty, &bindings)
            })
            .collect(),
    ))
}

pub(super) fn reject_unchecked_indexed_pattern(
    ctx: &mut InferCtx,
    result: &Ty,
    span: TextRange,
) -> Result<(), TypeError> {
    if result.con().is_some_and(|con| con.indexed_origin.is_some()) {
        return Err(invalid_index(
            ctx,
            span,
            "indexed constructor patterns require a direct case arm".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check, check_with_imports, collect_exports, ImportContext, ModuleExports};

    const VEC: &str = "pub type Vec<T, n :: Nat> do
  Empty -> Vec<T, 0>
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end\n";

    const FIRST: &str = "pub fn first<T, n :: Nat>(v :: Vec<T, Succ<n>>) -> T do
  case v do
    Push(head, _) -> head
  end
end\n";

    const COPY: &str = "pub fn copy<T, n :: Nat>(items :: Vec<T, n>) -> Vec<T, n> do
  case items do
    Empty -> Empty
    Push(head, tail) -> Push(head, copy(tail))
  end
end\n";

    fn checked(source: &str, imports: &ImportContext) -> crate::TypeckResult {
        let parse = mesh_parser::parse(source);
        assert!(parse.errors().is_empty(), "{:?}", parse.errors());
        check_with_imports(&parse, imports)
    }

    #[test]
    fn recursive_copy_retains_its_universal_index() {
        let source = format!(
            "{VEC}{COPY}
fn use_copy() do
  let empty :: Vec<String, 0> = copy(Empty)
  let one :: Vec<Int, 1> = copy(Push(1, Empty))
  let two :: Vec<Int, 2> = copy(Push(2, one))
  (empty, one, two)
end"
        );
        let result = checked(&source, &ImportContext::empty());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);

        let parse = mesh_parser::parse(&source);
        let exports = collect_exports(&parse, &result);
        let copy = &exports.functions["copy"];
        let Ty::Fun(params, ret) = &copy.ty else {
            panic!("expected a function");
        };
        assert_eq!(params[0], **ret);
        let Ty::App(_, args) = &params[0] else {
            panic!("expected an indexed family");
        };
        let Ty::Nat(NatIndex::Variable(index)) = args[1] else {
            panic!("expected a universally quantified index");
        };
        assert!(copy.vars.contains(&index));
        assert_eq!(copy.vars.len(), 2);
        assert_eq!(
            result.type_registry.runtime_type(&copy.ty),
            Ty::fun(
                vec![Ty::struct_ty("Vec", vec![args[0].clone()])],
                Ty::struct_ty("Vec", vec![args[0].clone()])
            )
        );
        let definition = parse
            .tree()
            .items()
            .find_map(|item| match item {
                mesh_parser::ast::item::Item::FnDef(def)
                    if def.name().and_then(|name| name.text()).as_deref() == Some("copy") =>
                {
                    Some(def)
                }
                _ => None,
            })
            .unwrap();
        let Ty::Fun(params, ret) = &result.types[&definition.syntax().text_range()] else {
            panic!("expected a checked function");
        };
        let body = definition.body().unwrap().tail_expr().unwrap();
        assert_eq!(result.types[&body.syntax().text_range()], **ret);
        let Ty::App(_, args) = &params[0] else {
            panic!("expected an indexed parameter");
        };
        let tails: Vec<_> = result
            .types
            .iter()
            .filter(|(range, _)| {
                &source[usize::from(range.start())..usize::from(range.end())] == "tail"
            })
            .map(|(_, ty)| ty)
            .collect();
        assert_eq!(
            tails.len(),
            2,
            "the pattern binding and the recursive argument"
        );
        for tail in tails {
            let Ty::App(_, tail_args) = tail else {
                panic!("expected a typed tail");
            };
            assert!(matches!(tail_args[1], Ty::Nat(NatIndex::Variable(_))));
            assert_ne!(tail_args[1], args[1]);
            assert_eq!(
                result.type_registry.runtime_type(tail),
                Ty::struct_ty("Vec", vec![args[0].clone()])
            );
        }
    }

    #[test]
    fn a_refined_case_can_infer_an_index_independent_result() {
        let source = format!(
            "{VEC}
fn empty_length(v :: Vec<Int, 0>) -> Int = 0
fn length<n :: Nat>(v :: Vec<Int, n>) -> Int do
  let size = case v do
    Empty -> empty_length(v)
    Push(_, tail) -> 1 + length(tail)
  end
  size
end
fn inferred<n :: Nat>(v :: Vec<Int, n>) do
  case v do
    Empty -> empty_length(v)
    Push(_, _) -> 1
  end
end"
        );
        let result = checked(&source, &ImportContext::empty());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    #[test]
    fn each_arm_must_satisfy_the_expected_index() {
        for (empty, push, expected) in [
            ("Push(0, Empty)", "v", "expected `0`, found `1`"),
            ("Empty", "Empty", "expected `Succ<k>`, found `0`"),
        ] {
            let source = format!(
                "{VEC}
fn bad<n :: Nat>(v :: Vec<Int, n>) -> Vec<Int, n> do
  case v do
    Empty -> {empty}
    Push(_, _) -> {push}
  end
end"
            );
            let result = checked(&source, &ImportContext::empty());
            assert!(
                result
                    .errors
                    .iter()
                    .any(|err| err.to_string().contains(expected)),
                "{:?}",
                result.errors
            );
        }
    }

    #[test]
    fn equations_are_gone_in_siblings_and_after_the_case_even_on_error() {
        let prefix = format!("{VEC}\nfn require_empty(v :: Vec<Int, 0>) -> Int = 0\n");
        let sibling = checked(
            &format!(
                "{prefix}
fn bad<n :: Nat>(v :: Vec<Int, n>) -> Int do
  case v do
    Empty -> require_empty(v)
    Push(_, _) -> require_empty(v)
  end
end"
            ),
            &ImportContext::empty(),
        );
        assert!(
            sibling
                .errors
                .iter()
                .any(|err| err.to_string().contains("expected `0`, found `Succ<k>`")),
            "{:?}",
            sibling.errors
        );
        for empty_body in ["require_empty(v)", "require_empty(Push(0, Empty))"] {
            let later = checked(
                &format!(
                    "{prefix}
fn bad<n :: Nat>(v :: Vec<Int, n>) -> Int do
  case v do
    Empty -> {empty_body}
    _ -> 1
  end
  case true do
    true -> require_empty(v)
    false -> 1
  end
end"
                ),
                &ImportContext::empty(),
            );
            assert!(
                later
                    .errors
                    .iter()
                    .any(|err| err.to_string().contains("expected `0`, found `n`")),
                "{:?}",
                later.errors
            );
        }
    }

    #[test]
    fn constructor_indices_cannot_escape_inferred_results_or_outer_bindings() {
        for body in [
            "fn bad<n :: Nat>(v :: Vec<Int, n>) -> Vec<Int, n> do
  let ignored = case v do
    Empty -> Empty
    Push(_, tail) -> tail
  end
  v
end",
            "fn bad<n :: Nat>(v :: Vec<Int, n>, consume) -> Int do
  case v do
    Empty -> 0
    Push(_, tail) ->
      consume(tail)
      0
  end
end",
            "fn bad<n :: Nat>(v :: Vec<Int, n>) do
  case v do
    Empty -> return Empty
    Push(_, tail) -> return tail
  end
end",
        ] {
            let result = checked(&format!("{VEC}{body}"), &ImportContext::empty());
            assert!(
                result
                    .errors
                    .iter()
                    .any(|err| err.to_string().contains("cannot escape its case arm")),
                "{body}: {:?}",
                result.errors
            );
        }
    }

    #[test]
    fn a_non_tail_case_joins_the_types_proved_by_each_arm() {
        let result = checked(
            "type Bit<n :: Nat> do
  Off -> Bit<0>
  On -> Bit<1>
end
fn bad<n :: Nat>(bit :: Bit<n>) -> Int do
  let ignored = case bit do
    Off -> bit
    On -> On
  end
  0
end",
            &ImportContext::empty(),
        );
        assert!(
            result
                .errors
                .iter()
                .any(|err| err.to_string().contains("expected `0`, found `1`")),
            "{:?}",
            result.errors
        );
    }

    #[test]
    fn refinement_is_scoped_even_if_an_earlier_arm_fixes_the_scrutinee_type() {
        let source = format!(
            "{VEC}
fn same_shape<n :: Nat>(v :: Vec<Int, n>, w :: Vec<Int, n>) -> Int = 0
fn bad<n :: Nat>(v :: Vec<Int, n>, candidate) -> Int do
  let escaped = case candidate do
    same when false ->
      same_shape(v, same)
      panic(\"unreachable\")
    Push(_, tail) -> tail
    _ -> panic(\"empty\")
  end
  0
end"
        );
        let result = checked(&source, &ImportContext::empty());
        assert!(
            result
                .errors
                .iter()
                .any(|err| err.to_string().contains("cannot escape its case arm")),
            "{:?}",
            result.errors
        );
    }

    #[test]
    fn an_inner_case_can_refine_the_outer_arms_constructor_index() {
        let prefix = format!(
            "{VEC}
fn single(v :: Vec<Int, 1>) -> Int = 1
fn nonempty<k :: Nat>(v :: Vec<Int, Succ<k>>) -> Int = 1\n"
        );
        for last in ["nonempty(v)", "single(v)"] {
            let source = format!(
                "{prefix}
fn size_class<n :: Nat>(v :: Vec<Int, n>) -> Int do
  case v do
    Empty -> 0
    Push(_, tail) ->
      let size = case tail do
        Empty -> single(v)
        Push(_, _) -> 2
      end
      size + {last}
  end
end"
            );
            let result = checked(&source, &ImportContext::empty());
            if last == "nonempty(v)" {
                assert!(result.errors.is_empty(), "{:?}", result.errors);
            } else {
                assert!(
                    result
                        .errors
                        .iter()
                        .any(|err| err.to_string().contains("found `Succ<k>`")),
                    "{:?}",
                    result.errors
                );
            }
        }
    }

    #[test]
    fn multi_index_patterns_refine_equalities_and_rule_out_conflicts() {
        let prefix = format!(
            "{VEC}
type Diag<a :: Nat, b :: Nat> do
  Equal<k :: Nat>(Vec<Int, k>) -> Diag<k, k>
  Offset<k :: Nat> -> Diag<k, Succ<k>>
end\n"
        );
        let source = format!(
            "{prefix}
fn preserve<a :: Nat, b :: Nat>(d :: Diag<a, b>, left :: Vec<Int, a>) -> Vec<Int, b> do
  case d do
    Equal(_) -> left
    Offset -> Push(1, left)
  end
end
fn equal_only<n :: Nat>(d :: Diag<n, n>) -> Int do
  case d do
    Equal(_) -> 0
  end
end
fn offset_only(d :: Diag<0, 1>) -> Int do
  case d do
    Offset -> 0
  end
end"
        );
        let result = checked(&source, &ImportContext::empty());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);

        let impossible = checked(
            &format!(
                "{prefix}
fn bad<n :: Nat>(d :: Diag<n, n>) -> Int do
  case d do
    Offset -> 0
    _ -> 1
  end
end"
            ),
            &ImportContext::empty(),
        );
        assert!(
            impossible
                .errors
                .iter()
                .any(|err| err.to_string().contains("cannot match")),
            "{:?}",
            impossible.errors
        );
    }

    #[test]
    fn constructors_infer_indices_and_first_covers_only_successor() {
        let source = format!(
            "{VEC}{FIRST}
fn main() do
  let one :: Vec<Int, 1> = Push(4, Empty)
  let two = Push(8, one)
  first(two)
end
"
        );
        let result = checked(&source, &ImportContext::empty());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        let two = result
            .types
            .values()
            .find(|ty| ty.to_string() == "Vec<Int, 2>")
            .expect("the constructor call retains its inferred index");
        assert_eq!(
            result.type_registry.runtime_type(two).to_string(),
            "Vec<Int>"
        );
        let exports = collect_exports(&mesh_parser::parse(&source), &result);
        assert_eq!(exports.functions["first"].vars.len(), 2);
        assert!(exports.functions["first"].ty.to_string().contains("Succ<"));
        let vec = &exports.sum_type_defs["Vec"];
        assert_eq!(vec.indexed.as_ref().unwrap().index_positions, [1]);
        let push = vec.variants[1].indexed.as_ref().unwrap();
        assert_eq!(push.index_params, ["k"]);
        assert_eq!(push.result.to_string(), "Vec<T, Succ<k>>");
    }

    #[test]
    fn wrong_sizes_and_matches_that_would_refine_signatures_are_rejected() {
        for (body, expected) in [
            (
                "fn bad() do
  let v :: Vec<Int, 0> = Push(1, Empty)
  v
end",
                "expected `0`, found `1`",
            ),
            (
                "fn bad<T, n :: Nat>(v :: Vec<T, Succ<n>>) do
  case v do
    Empty -> 1
    _ -> 0
  end
end",
                "cannot match",
            ),
            (
                "fn bad<T, n :: Nat>(v :: Vec<T, n>) do
  case v do
    Push(head, _) -> head
  end
end",
                "non-exhaustive",
            ),
            (
                "fn bad<T, n :: Nat>(v :: Vec<T, n>) -> Vec<T, n> do
  Empty
end",
                "expected `n`, found `0`",
            ),
            (
                "fn bad<T, n :: Nat>(v :: Vec<T, n>) -> Vec<T, Succ<n>> do
  v
end",
                "expected `Succ<n>`, found `n`",
            ),
            (
                "fn bad<T, n :: Nat>(v :: Vec<T, Succ<n>>) do
  case v do
    Push(_, Push(_, _)) -> 1
    _ -> 0
  end
end",
                "direct case arm",
            ),
            (
                "fn bad<T, n :: Nat>(v :: Vec<T, Succ<n>>) do
  case v do
    Push(_, _) when false -> 1
  end
end",
                "non-exhaustive",
            ),
        ] {
            let result = checked(&format!("{VEC}{body}"), &ImportContext::empty());
            assert!(
                result
                    .errors
                    .iter()
                    .any(|error| error.to_string().contains(expected)),
                "{body}: {:?}",
                result.errors
            );
        }
    }

    #[test]
    fn the_kind_boundary_rejects_type_arguments_in_nat_positions_and_vice_versa() {
        for (source, expected) in [
            (
                "fn bad(v :: Vec<Int, Int>) do v end",
                "expected a Nat index",
            ),
            ("fn bad(v :: Vec<0, 0>) do v end", "expected a type"),
            (
                "fn bad<T>(v :: Vec<Int, T>) do v end",
                "expected a Nat index",
            ),
            ("fn bad<n :: Nat>(v :: List<n>) do v end", "expected a type"),
            ("fn bad<n :: Nat>(v) -> n do v end", "expected a type"),
            (
                "fn bad<n :: Int>(v :: Vec<Int, n>) do v end",
                "must have kind",
            ),
            (
                "fn bad(v :: Vec<Int, 18446744073709551616>) do v end",
                "must fit",
            ),
            ("fn bad(v :: Vec) do v end", "requires 2 explicit arguments"),
            (
                "type Wrong<T, n :: Nat> do
  Wrong -> Vec<T, 0>
end",
                "must return its family",
            ),
            (
                "type Wrong<T, n :: Nat> do
  Wrong -> Wrong<Int, 0>
end",
                "must preserve",
            ),
            (
                "type Wrong<T, n :: Nat> do
  Wrong<k :: Nat>(Vec<T, k>) -> Wrong<T, 0>
end",
                "must appear in the result",
            ),
        ] {
            let result = checked(&format!("{VEC}{source}"), &ImportContext::empty());
            assert!(
                result
                    .errors
                    .iter()
                    .any(|error| error.to_string().contains(expected)),
                "{source}: {:?}",
                result.errors
            );
        }
    }

    #[test]
    fn exports_preserve_constructor_and_function_indices_and_identity() {
        let origin = "Shapes.Vectors";
        let producer = mesh_parser::parse(&format!("{VEC}{FIRST}"));
        assert!(producer.errors().is_empty(), "{:?}", producer.errors());
        let built = check_with_imports(
            &producer,
            &ImportContext {
                current_module: Some(origin.into()),
                ..ImportContext::empty()
            },
        );
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        let exports = collect_exports(&producer, &built);
        let imported = ImportContext {
            current_module: Some("Consumer".into()),
            module_exports: FxHashMap::from_iter([(
                origin.into(),
                ModuleExports::new(origin.into(), &exports),
            )]),
            module_qualifiers: FxHashMap::from_iter([("Vectors".into(), origin.into())]),
            ..ImportContext::empty()
        };

        let source = "import Shapes.Vectors
fn read(v :: Vectors.Vec<Int, 1>) -> Int do
  case v do
    Vectors.Push(head, _) -> head
  end
end
fn main() do
  let one :: Shapes.Vectors.Vec<Int, 1> = Vectors.Push(1, Vectors.Empty)
  Vectors.first(one)
end";
        let result = checked(source, &imported);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(result
            .types
            .values()
            .any(|ty| ty.to_string() == "Shapes.Vectors.Vec<Int, 1>"));

        let bad = checked(
            "import Shapes.Vectors
fn main() do
  let wrong :: Vectors.Vec<Int, 0> = Vectors.Push(1, Vectors.Empty)
  Vectors.first(wrong)
end",
            &imported,
        );
        assert!(
            bad.errors
                .iter()
                .any(|error| error.to_string().contains("expected `0`, found `1`")),
            "{:?}",
            bad.errors
        );

        let collision = checked(
            "import Shapes.Vectors
type Vec<T, n :: Nat> do
  LocalEmpty -> Vec<T, 0>
end",
            &imported,
        );
        assert!(
            collision
                .errors
                .iter()
                .any(|error| error.to_string().contains("shares a runtime type name")),
            "{:?}",
            collision.errors
        );
    }

    #[test]
    fn exact_indices_survive_tail_binding_aliases_and_independent_calls() {
        let source = format!(
            "{VEC}
type One<T> = Vec<T, 1>
type Succ<T> do
  Marker(T)
end
fn keep<T, n :: Nat>(v :: Vec<T, n>) -> Vec<T, n> = v
fn tail<T, n :: Nat>(v :: Vec<T, Succ<n>>) -> Vec<T, n> do
  case v do
    Push(_, rest) ->
      let same :: Vec<T, n> = rest
      keep(same)
  end
end
fn main() do
  let one :: One<Int> = Push(1, Empty)
  let zero :: Vec<Int, 0> = tail(one)
  let again :: One<Int> = keep(Push(2, keep(zero)))
  let ordinary :: Succ<Int> = Marker(4)
  again
end
"
        );
        let result = checked(&source, &ImportContext::empty());
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    #[test]
    fn standalone_exports_acquire_their_modules_nominal_identity() {
        let parse = mesh_parser::parse(&format!("{VEC}{FIRST}"));
        let built = check(&parse);
        assert!(built.errors.is_empty(), "{:?}", built.errors);
        let exports = collect_exports(&parse, &built);
        for name in ["Alpha", "Beta"] {
            let module = ModuleExports::new(name.into(), &exports);
            assert_eq!(
                module.sum_type_defs["Vec"].indexed.as_ref().unwrap().origin,
                name
            );
            let Ty::Fun(params, _) = &module.functions["first"].ty else {
                panic!("expected a function")
            };
            assert_eq!(
                params[0]
                    .con()
                    .unwrap()
                    .indexed_origin
                    .as_deref()
                    .map(String::as_str),
                Some(name)
            );
        }
    }

    #[test]
    fn nat_literal_comparisons_are_bounded_by_syntax() {
        let result = check(&mesh_parser::parse(&format!(
            "{VEC}
fn same(v :: Vec<Int, 18446744073709551615>) -> Vec<Int, 18446744073709551614> do
  v
end
"
        )));
        assert!(result
            .errors
            .iter()
            .any(|err| matches!(err, TypeError::Mismatch { .. })));
    }
}
