//! Correspondence between rigid and concrete lowerings of the same definition.

use std::collections::{HashMap, HashSet};

use mesh_codegen::mir::{
    lower::LoweredSignature, types::resolve_type, BinOp, MirExpr, MirFunction, MirLiteral,
    MirPattern, MirType, UnaryOp,
};
use mesh_typeck::{
    ty::{NatIndex, Ty, TyVar},
    TypeRegistry, VariantFieldInfo,
};

type Variant<'a> = (&'a MirFunction, &'a LoweredSignature);
type Variants<'a> = HashMap<&'a str, Variant<'a>>;

const ARM_WITNESS: &str = "\0mesh_correspondence_arm";

/// The supplied variants must have unambiguous definitions in their respective
/// modules. Check the whole runtime group so recursive edges cannot introduce
/// an unchecked body. The returned aliases are from the rigid lowering only.
pub(crate) fn check_specializations(
    template: Variant<'_>,
    template_variants: &[Variant<'_>],
    actual_variants: &[Variant<'_>],
    parameters: &[String],
    registry: &TypeRegistry,
) -> Result<HashSet<String>, String> {
    let templates = collect_variants(template.1, template_variants)?;
    let actuals = collect_variants(template.1, actual_variants)?;
    let mut aliases = HashSet::new();
    for &variant in template_variants {
        if variant.1.symbol != template.1.symbol
            && same_erased_type(&template.1.signature, &variant.1.signature)
        {
            check_pair(
                template,
                variant,
                parameters,
                registry,
                (&templates, &templates),
                true,
            )
            .map_err(|reason| format!("rigid variant {}: {reason}", variant.1.symbol))?;
            aliases.insert(variant.1.symbol.clone());
        }
    }
    for &variant in actual_variants {
        check_pair(
            template,
            variant,
            parameters,
            registry,
            (&templates, &actuals),
            false,
        )
        .map_err(|reason| format!("specialization {}: {reason}", variant.1.symbol))?;
    }
    Ok(aliases)
}

fn collect_variants<'a>(
    declaration: &LoweredSignature,
    variants: &[Variant<'a>],
) -> Result<Variants<'a>, String> {
    let mut symbols = HashMap::new();
    for &(function, entry) in variants {
        if entry.source_name != declaration.source_name
            || entry.source_file != declaration.source_file
            || entry.declaration != declaration.declaration
            || entry.symbol != function.name
        {
            return Err("parametric correspondence requires the same checked declaration".into());
        }
        if symbols
            .insert(entry.symbol.as_str(), (function, entry))
            .is_some()
        {
            return Err(format!("ambiguous checked variant {}", entry.symbol));
        }
    }
    Ok(symbols)
}

#[cfg(test)]
fn check(
    template: Variant<'_>,
    actual: Variant<'_>,
    parameters: &[String],
    registry: &TypeRegistry,
) -> Result<(), String> {
    collect_variants(template.1, &[actual])?;
    check_pair(
        template,
        actual,
        parameters,
        registry,
        (&HashMap::new(), &HashMap::new()),
        false,
    )
}

fn check_pair<'a>(
    (template_fn, template): Variant<'a>,
    (actual_fn, actual): Variant<'a>,
    parameters: &'a [String],
    registry: &'a TypeRegistry,
    (template_targets, actual_targets): (&Variants<'a>, &Variants<'a>),
    rigid_variant: bool,
) -> Result<(), String> {
    let rigid: HashSet<&str> = parameters.iter().map(String::as_str).collect();
    if rigid.len() != parameters.len()
        || rigid.contains(ARM_WITNESS)
        || rigid.iter().any(|name| !name.starts_with('\0'))
    {
        return Err("invalid rigid parameter names".into());
    }
    let mut substitution = HashMap::new();
    let concrete = HashSet::new();
    validate_type(&template.signature, &rigid, registry, false)?;
    validate_type(
        &actual.signature,
        if rigid_variant { &rigid } else { &concrete },
        registry,
        true,
    )?;
    if template.symbol != template_fn.name || actual.symbol != actual_fn.name {
        return Err("parametric correspondence requires checked MIR symbols".into());
    }
    unify(
        &template.signature,
        &actual.signature,
        &rigid,
        &mut substitution,
    )?;
    if substitution.len() != rigid.len() {
        return Err("signature does not determine every rigid parameter".into());
    }

    let (Ty::Fun(params, result), Ty::Fun(_, _)) = (&template.signature, &actual.signature) else {
        return Err("expected checked function signatures".into());
    };
    if params.len() != template_fn.params.len()
        || params.len() != actual_fn.params.len()
        || template_fn.is_closure_fn
        || actual_fn.is_closure_fn
        || !template_fn.captures.is_empty()
        || !actual_fn.captures.is_empty()
        || template_fn.has_tail_calls != actual_fn.has_tail_calls
    {
        return Err("parametric function has incompatible MIR function metadata".into());
    }

    if template.expression_types.len() != actual.expression_types.len() {
        return Err("specialization changed the checked expression-type keys".into());
    }
    let mut correspondence = Correspondence {
        substitution,
        rigid,
        registry,
        template_fn,
        actual_fn,
        lowering: template,
        actual_lowering: actual,
        template_targets,
        actual_targets,
        arm: None,
        visited_expressions: HashSet::new(),
        // Index by byte offsets so the compiler's TextRange need not be reconstructed.
        checked_expressions: HashMap::new(),
        locals: Vec::new(),
        decreasing: (0..params.len()).collect(),
        saw_tail_call: false,
    };
    for (range, template_ty) in &template.expression_types {
        let Some(actual_ty) = actual.expression_types.get(range) else {
            return Err("specialization changed the checked expression-type keys".into());
        };
        if correspondence.instantiate(template_ty) != *actual_ty {
            return Err(format!(
                "specialization changed the checked type at {range:?}"
            ));
        }
        correspondence
            .checked_expressions
            .insert((range.start().into(), range.end().into()), template_ty);
    }

    correspondence.require_abi(result, &template_fn.return_type, &actual_fn.return_type)?;
    for (index, ((template, actual), param)) in template_fn
        .params
        .iter()
        .zip(&actual_fn.params)
        .zip(params)
        .enumerate()
    {
        if template.0 != actual.0 {
            return Err("specialization changed a function parameter name".into());
        }
        correspondence.require_abi(param, &template.1, &actual.1)?;
        correspondence.locals.push((
            &template.0,
            Value {
                ty: param.clone(),
                origin: Some(Origin {
                    index,
                    smaller: false,
                }),
            },
        ));
    }
    correspondence.expression(&template_fn.body, &actual_fn.body, Some(result), 0)?;
    if correspondence.visited_expressions.len() != template.expression_types.len() {
        return Err("unvisited checked expression type in parametric correspondence".into());
    }
    if correspondence.saw_tail_call != template_fn.has_tail_calls {
        return Err("inconsistent MIR tail-call flag".into());
    }
    Ok(())
}

fn validate_type(
    ty: &Ty,
    rigid: &HashSet<&str>,
    registry: &TypeRegistry,
    allow_nat_variables: bool,
) -> Result<(), String> {
    let indexed_family = |name: &str| {
        let definition = registry.sum_type_defs.get(name)?;
        Some((definition, definition.indexed.as_ref()?))
    };
    match ty {
        Ty::Con(con)
            if con.indexed_origin.is_none()
                && indexed_family(&con.name).is_none()
                && (!con.name.starts_with('\0') || rigid.contains(con.name.as_str())) =>
        {
            return Ok(());
        }
        Ty::App(head, args) => {
            let Ty::Con(con) = head.as_ref() else {
                return Err(format!("unsupported checked type application: {ty:?}"));
            };
            if con.name.starts_with('\0') {
                return Err(format!("unsupported checked type application: {ty:?}"));
            }
            match (indexed_family(&con.name), &con.indexed_origin) {
                (None, None) => {}
                (Some((definition, indexed)), Some(origin))
                    if **origin == indexed.origin
                        && args.len() == definition.generic_params.len() =>
                {
                    for (position, arg) in args.iter().enumerate() {
                        if indexed.index_positions.contains(&position) {
                            let Ty::Nat(index) = arg else {
                                return Err(format!("expected a checked Nat index in {ty:?}"));
                            };
                            validate_index(index, rigid, allow_nat_variables)?;
                        } else {
                            validate_type(arg, rigid, registry, allow_nat_variables)?;
                        }
                    }
                    return Ok(());
                }
                _ => {
                    return Err(format!(
                        "missing exact checked indexed family origin or arity: {ty:?}"
                    ))
                }
            }
        }
        Ty::Fun(..) | Ty::Tuple(_) | Ty::Never => {}
        _ => {
            return Err(format!(
                "unsupported checked type in parametric correspondence: {ty:?}"
            ))
        }
    }
    for part in ty.parts() {
        validate_type(part, rigid, registry, allow_nat_variables)?;
    }
    Ok(())
}

fn validate_index(
    index: &NatIndex,
    rigid: &HashSet<&str>,
    allow_nat_variables: bool,
) -> Result<(), String> {
    match index {
        NatIndex::Literal(_) => Ok(()),
        NatIndex::Parameter(name) if rigid.contains(name.as_str()) => Ok(()),
        NatIndex::Variable(_) if allow_nat_variables => Ok(()),
        NatIndex::Succ(inner) => validate_index(inner, rigid, allow_nat_variables),
        _ => Err(format!(
            "unresolved checked Nat index {index:?}; branch-local indices require a proven substitution"
        )),
    }
}

type Substitution<'a> = HashMap<&'a str, Ty>;

fn bind<'a>(name: &'a str, actual: Ty, substitution: &mut Substitution<'a>) -> Result<(), String> {
    if let Some(bound) = substitution.get(name) {
        if *bound != actual {
            return Err(format!(
                "inconsistent substitution for rigid parameter {name:?}"
            ));
        }
    } else {
        substitution.insert(name, actual);
    }
    Ok(())
}

fn substitute(ty: &Ty, substitution: &Substitution<'_>) -> Ty {
    ty.replace_cons(&mut |con| {
        if con.indexed_origin.is_some() {
            return None;
        }
        substitution.get(con.name.as_str()).cloned()
    })
}

fn unify_index<'a>(
    template: &'a NatIndex,
    actual: NatIndex,
    rigid: &HashSet<&str>,
    substitution: &mut Substitution<'a>,
) -> Result<(), String> {
    match template {
        NatIndex::Parameter(name) if rigid.contains(name.as_str()) => {
            bind(name, Ty::Nat(actual), substitution)
        }
        NatIndex::Succ(inner) => {
            let predecessor = actual.predecessor().ok_or_else(|| {
                format!("cannot prove Nat index {actual:?} has the required successor shape")
            })?;
            unify_index(inner, predecessor, rigid, substitution)
        }
        _ if *template == actual => Ok(()),
        _ => Err(format!(
            "cannot establish exact Nat index equality {template:?} = {actual:?}"
        )),
    }
}

/// Bind only the given rigid leaves, without applying inference's coercions.
fn unify<'a>(
    template: &'a Ty,
    actual: &Ty,
    rigid: &HashSet<&str>,
    substitution: &mut Substitution<'a>,
) -> Result<(), String> {
    match (template, actual) {
        (Ty::Con(con), _) if con.indexed_origin.is_none() && rigid.contains(con.name.as_str()) => {
            bind(&con.name, actual.clone(), substitution)?;
        }
        (Ty::Nat(template), Ty::Nat(actual)) => {
            unify_index(template, actual.map_leaf(Clone::clone), rigid, substitution)?;
        }
        (Ty::Fun(params, _), Ty::Fun(concrete, _)) if params.len() == concrete.len() => {
            for (part, concrete) in template.parts().zip(actual.parts()) {
                unify(part, concrete, rigid, substitution)?;
            }
        }
        (Ty::Tuple(elems), Ty::Tuple(concrete)) if elems.len() == concrete.len() => {
            for (part, concrete) in elems.iter().zip(concrete) {
                unify(part, concrete, rigid, substitution)?;
            }
        }
        (Ty::App(head, args), Ty::App(concrete_head, concrete))
            if head == concrete_head && args.len() == concrete.len() =>
        {
            for (part, concrete) in args.iter().zip(concrete) {
                unify(part, concrete, rigid, substitution)?;
            }
        }
        _ if template == actual => {}
        _ => {
            return Err(format!(
                "checked type is not a rigid instantiation at {template:?} and {actual:?}"
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Origin {
    index: usize,
    smaller: bool,
}

#[derive(Clone)]
struct Value {
    ty: Ty,
    origin: Option<Origin>,
}

struct Correspondence<'a, 'targets> {
    substitution: Substitution<'a>,
    rigid: HashSet<&'a str>,
    registry: &'a TypeRegistry,
    template_fn: &'a MirFunction,
    actual_fn: &'a MirFunction,
    lowering: &'a LoweredSignature,
    actual_lowering: &'a LoweredSignature,
    template_targets: &'targets Variants<'a>,
    actual_targets: &'targets Variants<'a>,
    arm: Option<Arm<'a>>,
    visited_expressions: HashSet<(u32, u32)>,
    checked_expressions: HashMap<(u32, u32), &'a Ty>,
    locals: Vec<(&'a str, Value)>,
    decreasing: Vec<usize>,
    saw_tail_call: bool,
}

struct Arm<'a> {
    parameter: &'a str,
    index: NatIndex,
    has_witness: bool,
    checked_witness: Option<TyVar>,
}

impl Arm<'_> {
    fn refine(&self, ty: &Ty) -> Ty {
        substitute(
            ty,
            &HashMap::from([(self.parameter, Ty::Nat(self.index.clone()))]),
        )
    }
}

fn map_indices(ty: &Ty, f: &mut impl FnMut(&NatIndex) -> NatIndex) -> Ty {
    match ty {
        Ty::Nat(index) => Ty::Nat(index.map_leaf(f)),
        other => other.map_parts(|part| map_indices(part, f)),
    }
}

/// On validated types only: exact nominal identities and runtime arguments
/// survive, and only the Nat indices in their declared positions may differ.
fn same_erased_type(left: &Ty, right: &Ty) -> bool {
    match (left, right) {
        (Ty::Nat(_), Ty::Nat(_)) => true,
        (Ty::App(lh, la), Ty::App(rh, ra)) if lh == rh => same_erased_parts(la, ra),
        (Ty::Tuple(left), Ty::Tuple(right)) => same_erased_parts(left, right),
        (Ty::Fun(lp, lr), Ty::Fun(rp, rr)) => same_erased_parts(lp, rp) && same_erased_type(lr, rr),
        _ => left == right,
    }
}

fn same_erased_parts(left: &[Ty], right: &[Ty]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(l, r)| same_erased_type(l, r))
}

impl<'a> Correspondence<'a, '_> {
    fn instantiate(&self, template: &Ty) -> Ty {
        substitute(template, &self.substitution)
    }

    fn abi(&self, source: &Ty) -> MirType {
        match resolve_type(source, self.registry) {
            MirType::Tuple(_) => MirType::Ptr,
            ty => ty,
        }
    }

    fn require_abi(&self, source: &Ty, template: &MirType, actual: &MirType) -> Result<(), String> {
        let expected_template = self.abi(source);
        let expected_actual = self.abi(&self.instantiate(source));
        if template == &expected_template && actual == &expected_actual {
            Ok(())
        } else {
            Err(format!(
                "unjustified MIR type correspondence {template:?} to {actual:?} for {source:?}"
            ))
        }
    }

    fn checked_type(
        &mut self,
        expression: &MirExpr,
        expected: Option<&Ty>,
    ) -> Result<Option<Ty>, String> {
        let Some(span) = expression.source_span() else {
            return Ok(None);
        };
        let key = (span.span.start, span.span.end);
        let Some(ty) = self
            .checked_expressions
            .get(&key)
            .copied()
            .filter(|_| Some(span.file) == self.lowering.source_file)
        else {
            return Ok(None);
        };
        self.visited_expressions.insert(key);
        // Raw type equality between lowerings was checked at this source range.
        // Only an arm whose constructor introduces a witness can name it here.
        validate_type(
            ty,
            &self.rigid,
            self.registry,
            self.arm.as_ref().is_some_and(|arm| arm.has_witness),
        )
        .map_err(|reason| format!("checked template expression at {key:?}: {reason}"))?;
        let mut ty = ty.clone();
        if let Some(arm) = &mut self.arm {
            ty = arm.refine(&ty);
            if arm.has_witness {
                let mut witness = arm.checked_witness;
                ty = map_indices(&ty, &mut |leaf| match leaf {
                    NatIndex::Variable(var) if witness.get_or_insert(*var) == var => {
                        NatIndex::Parameter(ARM_WITNESS.into())
                    }
                    leaf => leaf.clone(),
                });
                if witness != arm.checked_witness {
                    if expected != Some(&ty) {
                        return Err(format!(
                            "checked Nat witness at {key:?} lacks an exact constructor-derived type"
                        ));
                    }
                    arm.checked_witness = witness;
                }
            }
        }
        self.validate_scoped_type(&ty)
            .map_err(|reason| format!("checked template expression at {key:?}: {reason}"))?;
        Ok(Some(ty))
    }

    fn validate_scoped_type(&self, ty: &Ty) -> Result<(), String> {
        if self.arm.as_ref().is_some_and(|arm| arm.has_witness) {
            let mut scoped = self.rigid.clone();
            scoped.insert(ARM_WITNESS);
            validate_type(ty, &scoped, self.registry, false)
        } else {
            validate_type(ty, &self.rigid, self.registry, false)
        }
    }

    fn local(&self, name: &str) -> Option<&Value> {
        self.locals
            .iter()
            .rev()
            .find(|(bound, _)| name == *bound)
            .map(|(_, value)| value)
    }

    fn expression(
        &mut self,
        template: &'a MirExpr,
        actual: &'a MirExpr,
        expected: Option<&Ty>,
        depth: usize,
    ) -> Result<Value, String> {
        if depth >= 256 {
            return Err("parametric MIR expression nesting is too deep".into());
        }
        if template.source_span() != actual.source_span() {
            return Err("specialization changed a MIR source span".into());
        }
        let refined = expected
            .zip(self.arm.as_ref())
            .map(|(ty, arm)| arm.refine(ty));
        let expected = refined.as_ref().or(expected);
        let checked = self.checked_type(template, expected)?;
        if let Some(checked) = &checked {
            require_context(checked, expected)?;
        }
        let expected = checked.as_ref().or(expected);
        let next = depth + 1;
        use MirExpr::*;
        let inferred = match (template, actual) {
            (IntLit(a, _), IntLit(b, _)) if a == b => Ty::int(),
            (FloatLit(a, _, _), FloatLit(b, _, _)) if a.to_bits() == b.to_bits() => Ty::float(),
            (BoolLit(a, _), BoolLit(b, _)) if a == b => Ty::bool(),
            (StringLit(a, _, _), StringLit(b, _, _)) if a == b => string_literal_type(expected),
            (Unit, Unit) => Ty::Tuple(Vec::new()),
            (Var(a, _), Var(b, _)) if a == b => {
                let value = self
                    .local(a)
                    .ok_or_else(|| format!("unsupported non-local MIR value {a:?}"))?;
                let mut value = value.clone();
                if let Some(arm) = &self.arm {
                    value.ty = arm.refine(&value.ty);
                }
                require_context(&value.ty, expected)?;
                self.require_abi(&value.ty, template.ty(), actual.ty())?;
                return Ok(value);
            }
            (
                UnaryOp {
                    op: a,
                    operand: left,
                    source_span: _,
                    ty: _,
                },
                UnaryOp {
                    op: b,
                    operand: right,
                    source_span: _,
                    ty: _,
                },
            ) if a == b => {
                let ty = match a {
                    self::UnaryOp::Neg => Ty::int(),
                    self::UnaryOp::Not => Ty::bool(),
                };
                self.expression(left, right, Some(&ty), next)?;
                ty
            }
            (
                BinOp {
                    op: a,
                    lhs: ll,
                    rhs: lr,
                    source_span: _,
                    ty: _,
                },
                BinOp {
                    op: b,
                    lhs: rl,
                    rhs: rr,
                    source_span: _,
                    ty: _,
                },
            ) if a == b => self.binary(*a, (ll, lr), (rl, rr), expected, next)?,
            (
                If {
                    cond: lc,
                    then_body: ly,
                    else_body: ln,
                    source_span: _,
                    ty: _,
                },
                If {
                    cond: rc,
                    then_body: ry,
                    else_body: rn,
                    source_span: _,
                    ty: _,
                },
            ) => {
                self.expression(lc, rc, Some(&Ty::bool()), next)?;
                let yes = self.expression(ly, ry, expected, next)?;
                self.expression(ln, rn, Some(&yes.ty), next)?.ty
            }
            (
                Let {
                    name: a,
                    value: lv,
                    body: lb,
                    ty: _,
                },
                Let {
                    name: b,
                    value: rv,
                    body: rb,
                    ty: _,
                },
            ) if a == b => {
                let value = self.expression(lv, rv, None, next)?;
                self.require_abi(&value.ty, template.ty(), actual.ty())?;
                self.locals.push((a, value));
                let result = self.expression(lb, rb, expected, next)?;
                self.locals.pop();
                // The stored type of a let is its initializer's type.
                return Ok(Value {
                    ty: result.ty,
                    origin: None,
                });
            }
            (Block(left, _), Block(right, _)) if left.len() == right.len() => {
                if let (Some((last, left)), Some((other, right))) =
                    (left.split_last(), right.split_last())
                {
                    for (left, right) in left.iter().zip(right) {
                        self.expression(left, right, None, next)?;
                    }
                    self.expression(last, other, expected, next)?.ty
                } else {
                    Ty::Tuple(Vec::new())
                }
            }
            (
                Match {
                    scrutinee: left,
                    arms: la,
                    source_span: _,
                    ty: _,
                },
                Match {
                    scrutinee: right,
                    arms: ra,
                    source_span: _,
                    ty: _,
                },
            ) if la.len() == ra.len() && !la.is_empty() => {
                let subject = self.expression(left, right, None, next)?;
                let mut result = expected.cloned();
                for (left, right) in la.iter().zip(ra) {
                    let scope = self.locals.len();
                    let refinement = self.arm_refinement(&subject.ty, &left.pattern)?;
                    let refined = refinement.is_some();
                    let mut branch_subject = subject.clone();
                    if let Some(arm) = refinement {
                        if self.arm.is_some() || checked.is_none() {
                            return Err("unsupported dependent MIR match scope".into());
                        }
                        branch_subject.ty = arm.refine(&subject.ty);
                        self.arm = Some(arm);
                    }
                    self.pattern(&left.pattern, &right.pattern, &branch_subject, next)?;
                    match (&left.guard, &right.guard) {
                        (Some(left), Some(right)) => {
                            self.expression(left, right, Some(&Ty::bool()), next)?;
                        }
                        (None, None) => {}
                        _ => {
                            return Err(
                                "specialization changed the presence of a match guard".into()
                            )
                        }
                    }
                    let body = self.expression(&left.body, &right.body, result.as_ref(), next)?;
                    if refined {
                        self.arm = None;
                    } else {
                        result = Some(body.ty);
                    }
                    self.locals.truncate(scope);
                }
                result.ok_or("empty MIR match")?
            }
            (
                ConstructVariant {
                    type_name: lt,
                    variant: lv,
                    fields: lf,
                    source_span: _,
                    ty: _,
                },
                ConstructVariant {
                    type_name: rt,
                    variant: rv,
                    fields: rf,
                    source_span: _,
                    ty: _,
                },
            ) if lv == rv && lf.len() == rf.len() => {
                let ty = expected.ok_or("missing exact checked type for MIR constructor")?;
                self.require_abi(
                    ty,
                    &MirType::SumType(lt.clone()),
                    &MirType::SumType(rt.clone()),
                )?;
                let fields = self.variant_fields(ty, lv)?;
                if fields.len() != lf.len() {
                    return Err(
                        "MIR constructor disagrees with the registered payload count".into(),
                    );
                }
                for ((left, right), field) in lf.iter().zip(rf).zip(&fields) {
                    self.expression(left, right, Some(field), next)?;
                }
                ty.clone()
            }
            (
                StructLit {
                    name: ln,
                    fields: lf,
                    source_span: _,
                    ty: _,
                },
                StructLit {
                    name: rn,
                    fields: rf,
                    source_span: _,
                    ty: _,
                },
            ) if lf.len() == rf.len() => {
                let ty = expected.ok_or("missing exact checked type for MIR struct literal")?;
                self.require_abi(
                    ty,
                    &MirType::Struct(ln.clone()),
                    &MirType::Struct(rn.clone()),
                )?;
                let fields = self.struct_fields(ty)?;
                let mut seen = HashSet::new();
                if fields.len() != lf.len() {
                    return Err(
                        "MIR struct literal disagrees with the registered field count".into(),
                    );
                }
                for ((ln, left), (rn, right)) in lf.iter().zip(rf) {
                    if ln != rn || !seen.insert(ln) {
                        return Err("incompatible MIR struct field sequence".into());
                    }
                    let field = fields
                        .iter()
                        .find(|(name, _)| name == ln)
                        .ok_or("unknown MIR struct field")?;
                    self.expression(left, right, Some(&field.1), next)?;
                }
                ty.clone()
            }
            (
                FieldAccess {
                    object: lo,
                    field: lf,
                    source_span: _,
                    ty: _,
                },
                FieldAccess {
                    object: ro,
                    field: rf,
                    source_span: _,
                    ty: _,
                },
            ) if lf == rf => {
                let value = self.expression(lo, ro, None, next)?;
                self.struct_fields(&value.ty)?
                    .into_iter()
                    .find(|(name, _)| name == lf)
                    .ok_or("unknown MIR field access")?
                    .1
            }
            (
                ListLit {
                    elements: left,
                    source_span: _,
                    ty: _,
                },
                ListLit {
                    elements: right,
                    source_span: _,
                    ty: _,
                },
            ) if left.len() == right.len() => {
                let mut element = expected.map(list_element).transpose()?.cloned();
                for (left, right) in left.iter().zip(right) {
                    let value = self.expression(left, right, element.as_ref(), next)?;
                    element = Some(value.ty);
                }
                Ty::list(element.ok_or("missing checked type for empty MIR list literal")?)
            }
            (
                Call {
                    func: lf,
                    args: la,
                    source_span: _,
                    ty: _,
                },
                Call {
                    func: rf,
                    args: ra,
                    source_span: _,
                    ty: _,
                },
            ) if la.len() == ra.len() => self.call((lf, la), (rf, ra), expected, next)?,
            (
                TailCall {
                    args: la,
                    ty: lt,
                    source_span: _,
                },
                TailCall {
                    args: ra,
                    ty: rt,
                    source_span: _,
                },
            ) if self.template_fn.has_tail_calls => {
                self.saw_tail_call = true;
                let Ty::Fun(_, result) = self.recursion(la, ra, next)? else {
                    unreachable!();
                };
                self.require_abi(&result, lt, rt)?;
                require_context(&result, expected)?;
                return Ok(Value {
                    ty: *result,
                    origin: None,
                });
            }
            _ => {
                return Err(format!(
                    "unsupported or changed MIR expression at {:?}",
                    template.source_span()
                ))
            }
        };

        require_context(&inferred, expected)?;
        self.require_abi(&inferred, template.ty(), actual.ty())?;
        Ok(Value {
            ty: inferred,
            origin: None,
        })
    }

    fn binary(
        &mut self,
        op: BinOp,
        (ll, lr): (&'a MirExpr, &'a MirExpr),
        (rl, rr): (&'a MirExpr, &'a MirExpr),
        expected: Option<&Ty>,
        depth: usize,
    ) -> Result<Ty, String> {
        use BinOp::*;
        if op == Concat {
            let ty = self.expression(ll, rl, expected, depth)?.ty;
            list_element(&ty)?;
            self.expression(lr, rr, Some(&ty), depth)?;
            return Ok(ty);
        }
        let operand = match (op, ll.ty()) {
            (And | Or | Eq | NotEq, MirType::Bool) => Ty::bool(),
            (Add | Sub | Mul | Eq | NotEq | Lt | Gt | LtEq | GtEq, MirType::Int) => Ty::int(),
            _ => {
                return Err(format!(
                    "unsupported parametric MIR binary operation {op:?}"
                ))
            }
        };
        self.expression(ll, rl, Some(&operand), depth)?;
        self.expression(lr, rr, Some(&operand), depth)?;
        Ok(match op {
            Add | Sub | Mul => operand,
            _ => Ty::bool(),
        })
    }

    fn arm_refinement(&self, ty: &Ty, pattern: &MirPattern) -> Result<Option<Arm<'a>>, String> {
        let Some((name, definition)) = ty
            .con_name()
            .and_then(|name| Some((name, self.registry.sum_type_defs.get(name)?)))
        else {
            return Ok(None);
        };
        let Some(family) = &definition.indexed else {
            return Ok(None);
        };
        let [position] = family.index_positions.as_slice() else {
            return Ok(None);
        };
        let Some(Ty::Nat(NatIndex::Parameter(parameter))) =
            ty.args_of(name).and_then(|args| args.get(*position))
        else {
            return Ok(None);
        };
        let Some(&parameter) = self.rigid.get(parameter.as_str()) else {
            return Ok(None);
        };
        let MirPattern::Constructor { variant, .. } = pattern else {
            return Err("dependent MIR match requires explicit constructor arms".into());
        };
        let schema = definition
            .variants
            .iter()
            .find(|entry| &entry.name == variant)
            .and_then(|entry| entry.indexed.as_ref())
            .ok_or("dependent MIR match lacks a checked constructor result")?;
        let Some(Ty::Nat(index)) = schema
            .result
            .args_of(name)
            .and_then(|args| args.get(*position))
        else {
            return Err("dependent MIR constructor lacks a checked result index".into());
        };
        let mut has_witness = false;
        let index = index.map_leaf(|leaf| match (leaf, schema.index_params.as_slice()) {
            (NatIndex::Parameter(name), [declared]) if name == declared => {
                has_witness = true;
                NatIndex::Parameter(ARM_WITNESS.into())
            }
            _ => leaf.clone(),
        });
        if usize::from(has_witness) != schema.index_params.len() {
            return Err("unsupported constructor Nat witnesses in dependent MIR match".into());
        }
        let parameters = if has_witness {
            HashSet::from([ARM_WITNESS])
        } else {
            HashSet::new()
        };
        validate_index(&index, &parameters, false)?;
        Ok(Some(Arm {
            parameter,
            index,
            has_witness,
            checked_witness: None,
        }))
    }

    fn variant_fields(&self, ty: &Ty, variant: &str) -> Result<Vec<Ty>, String> {
        let name = ty
            .con_name()
            .ok_or("missing checked sum type constructor")?;
        let definition = self
            .registry
            .sum_type_defs
            .get(name)
            .ok_or("unknown checked sum type")?;
        let variant = definition
            .variants
            .iter()
            .find(|def| def.name == variant)
            .ok_or("unknown variant in checked sum type")?;
        let fields = variant.fields.iter().map(|field| match field {
            VariantFieldInfo::Positional(ty) | VariantFieldInfo::Named(_, ty) => ty,
        });
        if let Some(indexed) = &variant.indexed {
            // Only constructor parameters are flexible here. A new equation for
            // a signature rigid would need checked evidence scoped to this arm.
            let parameters: HashSet<_> = definition
                .runtime_generic_params()
                .chain(&indexed.index_params)
                .map(String::as_str)
                .collect();
            let mut substitution = Substitution::new();
            unify(&indexed.result, ty, &parameters, &mut substitution).map_err(|reason| {
                format!(
                    "cannot prove checked index equations for constructor {name}.{}: {reason}",
                    variant.name
                )
            })?;
            if substitution.len() != parameters.len() {
                return Err(format!(
                    "checked index equations do not determine every parameter of {name}.{}",
                    variant.name
                ));
            }
            self.substituted_fields(fields, &substitution)
        } else if definition.indexed.is_some() {
            Err(format!(
                "missing checked index equations for constructor {name}.{}",
                variant.name
            ))
        } else {
            self.field_types(ty, name, &definition.generic_params, fields)
        }
    }

    fn struct_fields(&self, ty: &Ty) -> Result<Vec<(String, Ty)>, String> {
        let name = ty
            .con_name()
            .ok_or("missing checked struct type constructor")?;
        let definition = self
            .registry
            .struct_defs
            .get(name)
            .ok_or("unknown checked struct type")?;
        let types = self.field_types(
            ty,
            name,
            &definition.generic_params,
            definition.fields.iter().map(|(_, ty)| ty),
        )?;
        Ok(definition
            .fields
            .iter()
            .map(|(name, _)| name.clone())
            .zip(types)
            .collect())
    }

    fn field_types<'b>(
        &self,
        ty: &Ty,
        name: &str,
        parameters: &[String],
        fields: impl Iterator<Item = &'b Ty>,
    ) -> Result<Vec<Ty>, String> {
        let args = ty.args_of(name).unwrap_or_default();
        if parameters.len() != args.len() {
            return Err(format!("missing exact type arguments for {name:?}"));
        }
        let substitution = parameters
            .iter()
            .map(String::as_str)
            .zip(args.iter().cloned())
            .collect();
        self.substituted_fields(fields, &substitution)
    }

    fn substituted_fields<'b>(
        &self,
        fields: impl Iterator<Item = &'b Ty>,
        substitution: &Substitution<'_>,
    ) -> Result<Vec<Ty>, String> {
        fields
            .map(|ty| {
                let ty = substitute(ty, substitution);
                self.validate_scoped_type(&ty)?;
                Ok(ty)
            })
            .collect()
    }

    fn call(
        &mut self,
        (left, la): (&'a MirExpr, &'a [MirExpr]),
        (right, ra): (&'a MirExpr, &'a [MirExpr]),
        expected: Option<&Ty>,
        depth: usize,
    ) -> Result<Ty, String> {
        let (MirExpr::Var(ln, MirType::FnPtr(lp, lr)), MirExpr::Var(rn, MirType::FnPtr(rp, rr))) =
            (left, right)
        else {
            return Err("unsupported indirect MIR call".into());
        };
        if self.local(ln).is_some() || self.local(rn).is_some() {
            return Err("unsupported MIR call through a local binding".into());
        }
        if ln == &self.template_fn.name || self.template_targets.contains_key(ln.as_str()) {
            let result = self.recursion(la, ra, depth)?;
            let (params, ret) = match &result {
                Ty::Fun(params, ret) => (params, ret),
                _ => unreachable!("recursive call has a function signature"),
            };
            for ((lp, rp), param) in lp.iter().zip(rp).zip(params) {
                self.require_abi(param, lp, rp)?;
            }
            self.require_abi(ret, lr, rr)?;

            let checked_call = map_indices(&result, &mut |leaf| {
                if matches!(leaf, NatIndex::Parameter(name) if name == ARM_WITNESS) {
                    if let Some(witness) = self.arm.as_ref().and_then(|arm| arm.checked_witness) {
                        return NatIndex::Variable(witness);
                    }
                }
                leaf.clone()
            });
            self.require_recursive_target(
                ln,
                lp,
                lr,
                (self.template_fn, self.lowering),
                self.template_targets,
                &checked_call,
            )?;
            self.require_recursive_target(
                rn,
                rp,
                rr,
                (self.actual_fn, self.actual_lowering),
                self.actual_targets,
                &self.instantiate(&checked_call),
            )?;
            let Ty::Fun(_, result) = result else {
                unreachable!();
            };
            return Ok(*result);
        }
        if ln != rn || lp != rp || lr != rr || lp.len() != la.len() {
            return Err("specialization changed a MIR call target or intrinsic ABI".into());
        }

        // These operations allocate pure collection values. Codegen expands tuple
        // construction itself, and the list runtime preserves both input values.
        match (ln.as_str(), la, ra, lp.as_slice(), lr.as_ref()) {
            ("mesh_list_new", [], [], [], MirType::Ptr) => {
                let ty = expected.ok_or("missing checked type for empty list")?;
                list_element(ty)?;
                Ok(ty.clone())
            }
            (
                "mesh_list_concat",
                [ll, lr],
                [rl, rr],
                [MirType::Ptr, MirType::Ptr],
                MirType::Ptr,
            ) => self.binary(BinOp::Concat, (ll, lr), (rl, rr), expected, depth),
            ("__mesh_make_tuple", _, _, params, MirType::Ptr)
                if la.len() >= 2
                    && (params.iter().all(|ty| *ty == MirType::Int)
                        || (params.iter().eq(la.iter().map(MirExpr::ty))
                            && params.iter().eq(ra.iter().map(MirExpr::ty)))) =>
            {
                let hints = match expected {
                    Some(Ty::Tuple(fields)) if fields.len() == la.len() => Some(fields.as_slice()),
                    None => None,
                    _ => return Err("incompatible checked tuple intrinsic type".into()),
                };
                let mut fields = Vec::new();
                for (i, (left, right)) in la.iter().zip(ra).enumerate() {
                    let field = self.expression(left, right, hints.map(|hint| &hint[i]), depth)?;
                    fields.push(field.ty);
                }
                Ok(Ty::Tuple(fields))
            }
            _ => Err(format!("unsupported MIR call to {ln:?}")),
        }
    }

    fn require_recursive_target(
        &self,
        name: &str,
        params: &[MirType],
        result: &MirType,
        current: Variant<'a>,
        targets: &Variants<'a>,
        checked_call: &Ty,
    ) -> Result<(), String> {
        if name.starts_with("mesh_") || name.starts_with("__mesh_") {
            return Err(format!("unsupported recursive MIR target {name:?}"));
        }
        let (body, entry) = if name == current.0.name {
            if !same_erased_type(checked_call, &current.1.signature) {
                return Err(format!(
                    "recursive MIR target {name:?} changes runtime types"
                ));
            }
            current
        } else {
            let &(body, entry) = targets
                .get(name)
                .ok_or_else(|| format!("missing checked recursive MIR target {name:?}"))?;
            if &entry.signature != checked_call {
                return Err(format!(
                    "recursive MIR target {name:?} has a different checked call signature"
                ));
            }
            (body, entry)
        };
        if body.is_closure_fn
            || !body.captures.is_empty()
            || !params.iter().eq(body.params.iter().map(|(_, ty)| ty))
            || result != &body.return_type
        {
            return Err(format!(
                "incompatible MIR direct recursion ABI for {}",
                entry.symbol
            ));
        }
        Ok(())
    }

    /// Instantiate only the refined signature Nat, using a smaller argument's
    /// checked type. Recheck every argument against the resulting signature.
    fn recursion(
        &mut self,
        left: &'a [MirExpr],
        right: &'a [MirExpr],
        depth: usize,
    ) -> Result<Ty, String> {
        let Ty::Fun(params, result) = &self.lowering.signature else {
            unreachable!("the checked signature is a function");
        };
        if left.len() != params.len() || right.len() != params.len() {
            return Err("recursive MIR call has the wrong arity".into());
        }
        let mut substitution = Substitution::new();
        if let Some(arm) = &self.arm {
            let flexible = HashSet::from([arm.parameter]);
            for (index, (argument, param)) in left.iter().zip(params).enumerate() {
                let Some(value) = argument.var_name().and_then(|name| self.local(name)) else {
                    continue;
                };
                if matches!(value.origin, Some(origin) if origin.smaller && origin.index == index) {
                    unify(param, &arm.refine(&value.ty), &flexible, &mut substitution)?;
                }
            }
            if !substitution.contains_key(arm.parameter) {
                substitution.insert(arm.parameter, Ty::Nat(arm.index.clone()));
            }
        }
        let mut decreasing = Vec::new();
        let params = params
            .iter()
            .map(|param| substitute(param, &substitution))
            .collect::<Vec<_>>();
        for (index, ((left, right), param)) in left.iter().zip(right).zip(&params).enumerate() {
            let value = self.expression(left, right, Some(param), depth)?;
            if matches!(value.origin, Some(origin) if origin.smaller && origin.index == index) {
                decreasing.push(index);
            }
        }
        self.decreasing.retain(|index| decreasing.contains(index));
        if self.decreasing.is_empty() {
            return Err("MIR recursion must decrease a common structural parameter".into());
        }
        Ok(Ty::fun(params, substitute(result, &substitution)))
    }

    fn pattern(
        &mut self,
        template: &'a MirPattern,
        actual: &'a MirPattern,
        subject: &Value,
        depth: usize,
    ) -> Result<(), String> {
        if depth >= 256 {
            return Err("parametric MIR pattern nesting is too deep".into());
        }
        let next = depth + 1;
        let ty = &subject.ty;
        let smaller = subject.origin.map(|mut origin| {
            origin.smaller = true;
            origin
        });
        use MirPattern::*;
        match (template, actual) {
            (Wildcard, Wildcard) => {}
            (Var(a, lt), Var(b, rt)) if a == b => {
                self.require_abi(ty, lt, rt)?;
                self.locals.push((a, subject.clone()));
            }
            (Literal(a), Literal(b)) if same_literal(a, b, ty) => {}
            (
                Constructor {
                    type_name: lt,
                    variant: lv,
                    fields: lf,
                    bindings: lb,
                },
                Constructor {
                    type_name: rt,
                    variant: rv,
                    fields: rf,
                    bindings: rb,
                },
            ) if lv == rv && lf.len() == rf.len() && lb.len() == rb.len() => {
                self.require_abi(
                    ty,
                    &MirType::SumType(lt.clone()),
                    &MirType::SumType(rt.clone()),
                )?;
                let fields = self.variant_fields(ty, lv)?;
                if fields.len() != lf.len() {
                    return Err("MIR constructor pattern has the wrong payload count".into());
                }
                let scope = self.locals.len();
                for ((left, right), field) in lf.iter().zip(rf).zip(fields) {
                    let origin = same_erased_type(&field, ty).then_some(smaller).flatten();
                    self.pattern(left, right, &Value { ty: field, origin }, next)?;
                }
                if self.locals.len() - scope != lb.len() {
                    return Err("inconsistent MIR constructor pattern binding count".into());
                }
                for (((ln, lt), (rn, rt)), (name, value)) in
                    lb.iter().zip(rb).zip(&self.locals[scope..])
                {
                    if ln != name || rn != name {
                        return Err("incompatible MIR constructor pattern bindings".into());
                    }
                    self.require_abi(&value.ty, lt, rt)?;
                }
            }
            (Tuple(left), Tuple(right)) if left.len() == right.len() => {
                let Ty::Tuple(fields) = ty else {
                    return Err("missing checked tuple pattern type".into());
                };
                if fields.len() != left.len() {
                    return Err("MIR tuple pattern disagrees with its checked arity".into());
                }
                for ((left, right), field) in left.iter().zip(right).zip(fields) {
                    self.pattern(
                        left,
                        right,
                        &Value {
                            ty: field.clone(),
                            origin: None,
                        },
                        next,
                    )?;
                }
            }
            (
                Struct {
                    name: ln,
                    fields: lf,
                },
                Struct {
                    name: rn,
                    fields: rf,
                },
            ) if lf.len() == rf.len() => {
                self.require_abi(
                    ty,
                    &MirType::Struct(ln.clone()),
                    &MirType::Struct(rn.clone()),
                )?;
                let fields = self.struct_fields(ty)?;
                if fields.len() != lf.len() {
                    return Err(
                        "MIR struct pattern disagrees with the registered field count".into(),
                    );
                }
                for (((ln, lt, left), (rn, rt, right)), (name, field)) in
                    lf.iter().zip(rf).zip(fields)
                {
                    if *ln != name || *rn != name {
                        return Err("incompatible MIR struct pattern field order".into());
                    }
                    self.require_abi(&field, lt, rt)?;
                    self.pattern(
                        left,
                        right,
                        &Value {
                            ty: field,
                            origin: None,
                        },
                        next,
                    )?;
                }
            }
            (
                ListCons {
                    head: lh,
                    tail: ll,
                    elem_ty: lt,
                },
                ListCons {
                    head: rh,
                    tail: rl,
                    elem_ty: rt,
                },
            ) => {
                let element = list_element(ty)?;
                self.require_abi(element, lt, rt)?;
                self.pattern(
                    lh,
                    rh,
                    &Value {
                        ty: element.clone(),
                        origin: None,
                    },
                    next,
                )?;
                self.pattern(
                    ll,
                    rl,
                    &Value {
                        ty: ty.clone(),
                        origin: smaller,
                    },
                    next,
                )?;
            }
            (ListNil, ListNil) => {
                list_element(ty)?;
            }
            _ => return Err("unsupported or changed MIR pattern".into()),
        }
        Ok(())
    }
}

fn list_element(ty: &Ty) -> Result<&Ty, String> {
    match ty.args_of("List") {
        Some([element]) => Ok(element),
        _ => Err("expected an exact checked list type".into()),
    }
}

fn require_context(inferred: &Ty, expected: Option<&Ty>) -> Result<(), String> {
    if let Some(expected) = expected.filter(|&expected| expected != inferred) {
        return Err(format!(
            "MIR expression has type {inferred:?}, expected {expected:?}"
        ));
    }
    Ok(())
}

fn string_literal_type(expected: Option<&Ty>) -> Ty {
    match expected {
        // Atoms also lower to a string literal.
        Some(ty @ Ty::Con(con)) if con.name == "Atom" => ty.clone(),
        _ => Ty::string(),
    }
}

fn same_literal(left: &MirLiteral, right: &MirLiteral, ty: &Ty) -> bool {
    match (left, right) {
        (MirLiteral::Int(a), MirLiteral::Int(b)) => a == b && *ty == Ty::int(),
        (MirLiteral::Bool(a), MirLiteral::Bool(b)) => a == b && *ty == Ty::bool(),
        (MirLiteral::Float(a), MirLiteral::Float(b)) => {
            a.to_bits() == b.to_bits() && *ty == Ty::float()
        }
        (MirLiteral::String(a), MirLiteral::String(b)) => {
            a == b && *ty == string_literal_type(Some(ty))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_codegen::mir::{lower::lower_module_to_mir_with_signatures, MirSourceId};
    use mesh_typeck::ty::TyCon;

    const RIGID: &str = "\0mesh_parametric_test_T";
    const RIGID_INDEX: &str = "\0mesh_index_1";

    const VEC: &str = r#"
type Vec<T, n :: Nat> do
  Empty -> Vec<T, 0>
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end
"#;

    const MIRROR: &str = r#"
type Tree<T> do
  Leaf(T)
  Node(Tree<T>, Tree<T>)
end

fn mirror<T>(tree :: Tree<T>) -> Tree<T> do
  if true do 7 else 8 end
  case tree do
    Leaf(value) -> Leaf(value)
    Node(left, right) -> Node(mirror(right), mirror(left))
  end
end
"#;

    fn mirror_signature(element: Ty) -> Ty {
        let tree = Ty::App(Box::new(Ty::Con(TyCon::new("Tree"))), vec![element]);
        Ty::fun(vec![tree.clone()], tree)
    }

    fn lower(
        source: &str,
        name: &str,
        specializations: Vec<Ty>,
    ) -> (Vec<(MirFunction, LoweredSignature)>, TypeRegistry) {
        let parse = mesh_parser::parse(source);
        assert!(parse.errors().is_empty(), "{:?}", parse.errors());
        let checked = mesh_typeck::check(&parse);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let injected = HashMap::from([(name.into(), specializations)]);
        let (module, signatures) = lower_module_to_mir_with_signatures(
            &parse,
            &checked,
            "",
            &HashSet::new(),
            &injected,
            &[],
            |_| Some(MirSourceId(3)),
        )
        .unwrap();
        let variants = signatures
            .into_iter()
            .filter(|entry| entry.source_name == name)
            .map(|entry| {
                let function = module
                    .functions
                    .iter()
                    .find(|f| f.name == entry.symbol)
                    .unwrap();
                (function.clone(), entry)
            })
            .collect();
        (variants, checked.type_registry)
    }

    fn lower_mirror(elements: Vec<Ty>) -> (Vec<(MirFunction, LoweredSignature)>, TypeRegistry) {
        lower(
            MIRROR,
            "mirror",
            elements.into_iter().map(mirror_signature).collect(),
        )
    }

    fn vector(registry: &TypeRegistry, element: Ty, index: NatIndex) -> Ty {
        Ty::App(
            Box::new(Ty::Con(registry.sum_type_defs["Vec"].head())),
            vec![element, Ty::Nat(index)],
        )
    }

    #[test]
    fn native_known_successor_variants_match_ground_indices() {
        let source = format!(
            "{VEC}{}",
            r#"
fn first<T, n :: Nat>(items :: Vec<T, Succ<n>>) -> T do
  case items do
    Push(head, _) -> head
  end
end
fn rest<T, n :: Nat>(items :: Vec<T, Succ<n>>) -> Vec<T, n> do
  case items do
    Push(_, tail) -> tail
  end
end
fn exercise(one :: Vec<Int, 1>, two :: Vec<Bool, 2>) =
  (first(one), first(two), rest(one), rest(two))
"#
        );
        for name in ["first", "rest"] {
            let (actual, registry) = lower(&source, name, vec![]);
            assert_eq!(actual.len(), 2);
            let element = Ty::Con(TyCon::new(RIGID));
            let n = NatIndex::Parameter(RIGID_INDEX.into());
            let input = vector(
                &registry,
                element.clone(),
                NatIndex::Succ(Box::new(n.clone())),
            );
            let output = if name == "first" {
                element
            } else {
                vector(&registry, element, n)
            };
            let rigid = Ty::fun(vec![input], output);
            let (template, _) = lower(&source, name, vec![rigid.clone()]);
            let (template_fn, template_ty) = template
                .iter()
                .find(|(_, entry)| entry.signature == rigid)
                .unwrap();
            let parameters = [RIGID.into(), RIGID_INDEX.into()];
            for (actual_fn, actual_ty) in &actual {
                let compare = |actual_fn: &MirFunction,
                               actual_ty: &LoweredSignature,
                               registry: &TypeRegistry| {
                    check(
                        (template_fn, template_ty),
                        (actual_fn, actual_ty),
                        &parameters,
                        registry,
                    )
                };
                compare(actual_fn, actual_ty, &registry)
                    .unwrap_or_else(|reason| panic!("{}: {reason}", actual_ty.symbol));

                if name == "rest" {
                    // The returned vector's layout is unchanged when its index changes.
                    let mut changed = actual_ty.clone();
                    let range_ty = changed
                        .expression_types
                        .values_mut()
                        .find(|ty| ty.con_name() == Some("Vec"))
                        .unwrap();
                    let Ty::App(_, args) = range_ty else {
                        unreachable!();
                    };
                    args[1] = Ty::Nat(NatIndex::Literal(3));
                    assert!(compare(actual_fn, &changed, &registry)
                        .unwrap_err()
                        .contains("changed the checked type"));

                    let mut wrong_fields = registry.clone();
                    let push = &mut wrong_fields.sum_type_defs.get_mut("Vec").unwrap().variants[1];
                    let VariantFieldInfo::Positional(Ty::App(_, args)) = &mut push.fields[1] else {
                        panic!("expected the checked tail field");
                    };
                    args[1] = Ty::Nat(NatIndex::Literal(3));
                    assert!(compare(actual_fn, actual_ty, &wrong_fields)
                        .unwrap_err()
                        .contains("MIR expression has type"));
                }

                let mut wrong_registry = registry.clone();
                wrong_registry
                    .sum_type_defs
                    .get_mut("Vec")
                    .unwrap()
                    .indexed
                    .as_mut()
                    .unwrap()
                    .origin = "Other".into();
                assert!(compare(actual_fn, actual_ty, &wrong_registry)
                    .unwrap_err()
                    .contains("exact checked indexed family origin"));

                let mut changed = actual_fn.clone();
                let body = match &mut changed.body {
                    MirExpr::Block(steps, _) => steps.last_mut().unwrap(),
                    expr => expr,
                };
                let MirExpr::Match { arms, .. } = body else {
                    panic!("expected a lowered case");
                };
                let MirPattern::Constructor { bindings, .. } = &mut arms[0].pattern else {
                    panic!("expected a lowered constructor");
                };
                bindings[0].1 = MirType::Unit;
                assert!(compare(&changed, actual_ty, &registry)
                    .unwrap_err()
                    .contains("unjustified MIR type correspondence"));
            }
        }
    }

    #[test]
    fn native_index_changing_recursion_compares_every_emitted_body() {
        let source = format!(
            "{VEC}{}",
            r#"
fn copy<T, n :: Nat>(items :: Vec<T, n>) -> Vec<T, n> do
  case items do
    Empty -> Empty
    Push(head, tail) -> Push(head, copy(tail))
  end
end
fn exercise(zero :: Vec<Int, 0>, two :: Vec<Int, 2>, yes :: Vec<Bool, 1>) =
  (copy(zero), copy(two), copy(yes))
"#
        );
        let checked = mesh_typeck::check(&mesh_parser::parse(&source));
        let signature = |element, index| {
            let vec = vector(&checked.type_registry, element, index);
            Ty::fun(vec![vec.clone()], vec)
        };
        let rigid = signature(
            Ty::Con(TyCon::new(RIGID)),
            NatIndex::Parameter(RIGID_INDEX.into()),
        );
        let ground = signature(Ty::int(), NatIndex::Literal(2));
        let (template, registry) = lower(&source, "copy", vec![rigid.clone()]);
        let (actual, _) = lower(&source, "copy", vec![ground.clone()]);
        let (template_fn, template_ty) = template
            .iter()
            .find(|(_, entry)| entry.signature == rigid)
            .unwrap();
        let rigid_bodies: Vec<_> = template.iter().map(|(f, entry)| (f, entry)).collect();
        let template = (template_fn, template_ty);
        assert_eq!(actual.len(), 5);

        fn pairs(variants: &[(MirFunction, LoweredSignature)]) -> Vec<Variant<'_>> {
            variants.iter().map(|(f, entry)| (f, entry)).collect()
        }
        fn recursive_call(function: &mut MirFunction) -> &mut MirExpr {
            let MirExpr::Match { arms, .. } = &mut function.body else {
                panic!("expected the lowered copy match");
            };
            let MirExpr::ConstructVariant { fields, .. } = &mut arms[1].body else {
                panic!("expected the lowered Push");
            };
            &mut fields[1]
        }
        let parameters = [RIGID.into(), RIGID_INDEX.into()];
        let compare = |actual: &[(MirFunction, LoweredSignature)]| {
            check_specializations(
                template,
                &rigid_bodies,
                &pairs(actual),
                &parameters,
                &registry,
            )
        };
        let aliases = compare(&actual).unwrap();
        assert_eq!(aliases.len(), 1);

        let Ty::Fun(params, _) = &ground else {
            unreachable!();
        };
        let mut changed = actual.clone();
        let (body, _) = changed
            .iter_mut()
            .find(|(_, entry)| entry.signature == ground)
            .unwrap();
        let current = body.name.clone();
        let MirExpr::Call { func, .. } = recursive_call(body) else {
            panic!("expected the indexed recursive call");
        };
        let MirExpr::Var(target, _) = func.as_mut() else {
            panic!("expected a direct recursive call target");
        };
        let other_name = target.clone();
        assert_ne!(other_name, current);
        let mut missing = actual.clone();
        missing.retain(|(_, entry)| entry.symbol != other_name);
        assert!(compare(&missing)
            .unwrap_err()
            .contains("missing checked recursive MIR target"));

        let empty_vec = vector(&registry, Ty::int(), NatIndex::Literal(0));
        let empty_copy = Ty::fun(vec![empty_vec.clone()], empty_vec);
        let (_, empty) = actual
            .iter()
            .find(|(_, entry)| entry.signature == empty_copy)
            .unwrap();
        *target = empty.symbol.clone();
        assert!(compare(&changed)
            .unwrap_err()
            .contains("different checked call signature"));

        let mut changed = actual.clone();
        let (body, _) = changed
            .iter_mut()
            .find(|(_, entry)| entry.signature == ground)
            .unwrap();
        let MirExpr::Call { func, .. } = recursive_call(body) else {
            unreachable!();
        };
        let MirExpr::Var(_, MirType::FnPtr(_, ret)) = func.as_mut() else {
            unreachable!();
        };
        **ret = MirType::Unit;
        assert!(compare(&changed)
            .unwrap_err()
            .contains("unjustified MIR type correspondence"));

        let mut changed = actual.clone();
        let (body, entry) = changed
            .iter_mut()
            .find(|(_, entry)| entry.signature == ground)
            .unwrap();
        let span = recursive_call(body).source_span().unwrap().span;
        let (_, ty) = entry
            .expression_types
            .iter_mut()
            .find(|(range, _)| {
                u32::from(range.start()) == span.start && u32::from(range.end()) == span.end
            })
            .unwrap();
        let Ty::App(_, args) = ty else {
            panic!("expected the checked Vec call result");
        };
        let Ty::Nat(NatIndex::Variable(witness)) = &mut args[1] else {
            panic!("expected the constructor-local checked witness");
        };
        witness.0 += 1;
        assert!(compare(&changed)
            .unwrap_err()
            .contains("changed the checked type"));

        let mut changed = actual.clone();
        let (dependent_body, _) = changed
            .iter_mut()
            .find(|(_, entry)| entry.symbol == other_name)
            .unwrap();
        let MirExpr::Call { args, .. } = recursive_call(dependent_body) else {
            unreachable!();
        };
        args[0] = MirExpr::Var("items".into(), resolve_type(&params[0], &registry));
        let error = compare(&changed).unwrap_err();
        assert!(
            error.contains(&other_name) && error.contains("unsupported or changed MIR expression"),
            "{error}"
        );
    }

    #[test]
    fn native_generic_mirror_matches_three_specializations() {
        let source = format!(
            "{MIRROR}{}",
            r#"
fn exercise(tree :: Tree<Int>, ints :: Tree<List<Int>>, bools :: Tree<List<Bool>>) =
  (mirror(tree), mirror(ints), mirror(bools))
"#
        );
        let rigid = mirror_signature(Ty::Con(TyCon::new(RIGID)));
        let (template, registry) = lower(&source, "mirror", vec![rigid.clone()]);
        let (actual, _) = lower(&source, "mirror", vec![]);
        assert_eq!(actual.len(), 3);
        let (template_fn, template_ty) = template
            .iter()
            .find(|(_, entry)| entry.signature == rigid)
            .unwrap();
        for (actual_fn, actual_ty) in &actual {
            check(
                (template_fn, template_ty),
                (actual_fn, actual_ty),
                &[RIGID.into()],
                &registry,
            )
            .unwrap_or_else(|reason| panic!("{}: {reason}", actual_ty.symbol));
        }
        let (_, original) = actual
            .iter()
            .find(|(_, entry)| entry.signature == mirror_signature(Ty::list(Ty::int())))
            .unwrap();
        let (other_fn, other) = actual
            .iter()
            .find(|(_, entry)| entry.signature == mirror_signature(Ty::list(Ty::bool())))
            .unwrap();
        let mut corrupted = other.clone();
        corrupted.expression_types = original.expression_types.clone();
        let result = check(
            (template_fn, template_ty),
            (other_fn, &corrupted),
            &[RIGID.into()],
            &registry,
        );
        assert!(result.unwrap_err().contains("changed the checked type"));
    }

    #[test]
    fn native_mirror_rejects_a_changed_discarded_branch_or_recursive_argument() {
        let (template, registry) = lower_mirror(vec![Ty::Con(TyCon::new(RIGID))]);
        let (actual, _) = lower_mirror(vec![Ty::int()]);
        let (template_fn, template_ty) = &template[0];
        let (actual_fn, actual_ty) = &actual[0];
        let compare = |actual_fn| {
            check(
                (template_fn, template_ty),
                (actual_fn, actual_ty),
                &[RIGID.into()],
                &registry,
            )
        };
        compare(actual_fn).unwrap();

        let mut changed = actual_fn.clone();
        let MirExpr::Block(steps, _) = &mut changed.body else {
            panic!("expected a native block: {:?}", changed.body);
        };
        let MirExpr::If { else_body, .. } = &mut steps[0] else {
            panic!("expected a discarded branch");
        };
        **else_body = MirExpr::IntLit(9, MirType::Int);
        assert!(compare(&changed).is_err());

        let mut changed = actual_fn.clone();
        let MirExpr::Block(steps, _) = &mut changed.body else {
            unreachable!();
        };
        let MirExpr::Match { arms, .. } = &mut steps[1] else {
            panic!("expected a match");
        };
        let MirExpr::ConstructVariant { fields, .. } = &mut arms[1].body else {
            panic!("expected a variant");
        };
        fields.swap(0, 1);
        assert!(compare(&changed).is_err());
    }

    #[test]
    fn native_tail_recursion_requires_a_smaller_argument() {
        let source = r#"
fn last<T>(items :: List<T>) -> List<T> do
  case items do
    [] -> []
    _ :: rest -> last(rest)
  end
end
"#;
        let signature = |element| {
            let list = Ty::list(element);
            Ty::fun(vec![list.clone()], list)
        };
        for (source, valid) in [
            (source.to_owned(), true),
            (source.replace("last(rest)", "last(items)"), false),
        ] {
            let (template, registry) =
                lower(&source, "last", vec![signature(Ty::Con(TyCon::new(RIGID)))]);
            let (actual, _) = lower(&source, "last", vec![signature(Ty::int())]);
            let (template_fn, template_ty) = &template[0];
            let (actual_fn, actual_ty) = &actual[0];
            assert!(template_fn.has_tail_calls && actual_fn.has_tail_calls);
            let result = check(
                (template_fn, template_ty),
                (actual_fn, actual_ty),
                &[RIGID.into()],
                &registry,
            );
            if valid {
                result.unwrap();
            } else {
                assert!(result
                    .unwrap_err()
                    .contains("must decrease a common structural parameter"));
            }
        }
    }

    #[test]
    fn native_specialization_rejects_an_unchanged_effect_inside_a_discarded_step() {
        let source = r#"
fn keep<T>(value :: T) -> T do
  println("effect inside a discarded step")
  value
end
"#;
        let signature = |ty: Ty| Ty::fun(vec![ty.clone()], ty);
        let (template, registry) =
            lower(source, "keep", vec![signature(Ty::Con(TyCon::new(RIGID)))]);
        let (actual, _) = lower(source, "keep", vec![signature(Ty::int())]);
        let (template_fn, template_ty) = &template[0];
        let (actual_fn, actual_ty) = &actual[0];
        let result = check(
            (template_fn, template_ty),
            (actual_fn, actual_ty),
            &[RIGID.into()],
            &registry,
        );
        assert!(result.unwrap_err().contains("unsupported MIR call"));
    }
}
