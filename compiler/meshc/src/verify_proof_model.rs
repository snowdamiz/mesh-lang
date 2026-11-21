//! Translate executable raw MIR into a closed fragment of Lean.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write as _;

use mesh_codegen::mir::lower::LoweredSignature;
use mesh_codegen::mir::types::{mangle_type_name, resolve_type};
use mesh_codegen::mir::{
    sum_type_base, BinOp, MirExpr, MirFunction, MirLiteral, MirMatchArm, MirModule, MirPattern,
    MirSourceId, MirType, UnaryOp,
};
use mesh_typeck::ty::{NatIndex, Ty, TyCon, TyVar};
use mesh_typeck::{TypeRegistry, VariantFieldInfo};

use crate::verify_solver::EncodingError;

pub(super) struct KernelModel {
    pub definitions: String,
    pub parameter_types: Vec<String>,
}

/// Symbols whose native calls or argument ABI can change if a project
/// declaration shadows them. Validate these before translating a project.
pub(super) const TRUSTED_BUILTINS: &[&str] = &[
    "mesh_gc_alloc_actor",
    "mesh_list_new",
    "mesh_list_from_array",
    "mesh_list_concat",
    "mesh_list_length",
    "mesh_list_head",
    "mesh_list_tail",
    "__mesh_make_tuple",
    "mesh_tuple_first",
    "mesh_tuple_second",
    "mesh_tuple_nth",
    "mesh_tuple_size",
];

#[cfg(test)]
pub(super) fn translate(
    target: &MirFunction,
    expected: &MirFunction,
    signature: &Ty,
) -> Result<KernelModel, EncodingError> {
    let registry = TypeRegistry::default();
    ModelSession::new(&[], &[]).translate(target, &registry, expected, &registry, signature)
}

#[cfg(test)]
pub(super) fn translate_definition(
    function: &MirFunction,
    signature: &Ty,
    symbol: &str,
) -> Result<String, EncodingError> {
    ModelSession::new(&[], &[]).definition(function, signature, &TypeRegistry::default(), symbol)
}

/// Source ownership comes from the checked project's declarations, including
/// private declarations. Imported copies in registries have no owner ID.
/// This is an ambiguity check. The caller resolves names and visibility in
/// each source's checked scope before handing its types to the model.
pub(crate) struct NominalDeclaration<'a> {
    pub name: String,
    pub registry: &'a TypeRegistry,
}

/// One uniquely resolved top-level MIR body and its exact checked signature.
#[derive(Clone, Copy)]
pub(crate) struct CheckedCallee<'a> {
    pub function: &'a MirFunction,
    pub signature: &'a Ty,
    pub registry: &'a TypeRegistry,
    pub lowering: Option<&'a LoweredSignature>,
}

impl<'a> CheckedCallee<'a> {
    fn checked_lowering(self) -> Result<Option<&'a LoweredSignature>, EncodingError> {
        if self.lowering.is_some_and(|entry| {
            entry.symbol != self.function.name || &entry.signature != self.signature
        }) {
            return Err(format!("conflicting checked binding for `{}`", self.function.name).into());
        }
        Ok(self.lowering)
    }
}

#[derive(Clone)]
struct Helper {
    symbol: String,
    params: Vec<ModelType>,
    result: ModelType,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DefinitionScope {
    Project,
    StandaloneLaw,
    Parametric,
}

/// One Lean artifact's shared nominal declarations. Registries must come from
/// the checked contexts that produced the types and functions being encoded.
/// The caller checks native function resolution and builtin shadowing before
/// importing any body.
pub(crate) struct ModelSession<'a> {
    modules: &'a [MirModule],
    declarations: &'a [NominalDeclaration<'a>],
    nominals: BTreeMap<String, NominalDef>,
    families: BTreeMap<String, CheckedNominal>,
    active_families: Vec<String>,
    prelude: String,
    callees: BTreeMap<&'a str, CheckedCallee<'a>>,
    active_functions: Vec<String>,
    helpers: BTreeMap<&'a str, Helper>,
    next_helper: usize,
    helper_definitions: String,
}

impl<'a> ModelSession<'a> {
    pub(crate) fn new(
        modules: &'a [MirModule],
        declarations: &'a [NominalDeclaration<'a>],
    ) -> Self {
        Self {
            modules,
            declarations,
            nominals: BTreeMap::new(),
            families: BTreeMap::new(),
            active_families: Vec::new(),
            prelude: String::new(),
            callees: BTreeMap::new(),
            active_functions: Vec::new(),
            helpers: BTreeMap::new(),
            next_helper: 0,
            helper_definitions: String::new(),
        }
    }

    pub(crate) fn with_callees(mut self, callees: BTreeMap<&'a str, CheckedCallee<'a>>) -> Self {
        self.callees = callees;
        self
    }

    /// Prepend this once, after translating every import in the artifact.
    pub(crate) fn prelude(&self) -> &str {
        &self.prelude
    }

    /// Append after the type prelude and before the requested definitions.
    pub(crate) fn helper_definitions(&self) -> &str {
        &self.helper_definitions
    }

    /// Resolve explicit imports through the same cache as calls from checked code.
    pub(crate) fn import(&mut self, name: &str) -> Result<String, EncodingError> {
        Ok(self.helper(name, 0)?.symbol)
    }

    /// Render the checked type, including arguments to a generic family.
    pub(crate) fn source_type(
        &mut self,
        ty: &Ty,
        registry: &TypeRegistry,
    ) -> Result<String, EncodingError> {
        Ok(TypeScope::new(self, registry).source(ty)?.lean())
    }

    /// Validate the native nominal layouts used by an indexed specialization.
    /// Its checked Nat variables remain distinct even though native code erases
    /// them. Other nominal instances in the specialization need the same ABI
    /// checks as the indexed ones.
    pub(crate) fn validate_indexed_native_layouts(
        &mut self,
        entry: &LoweredSignature,
        registry: &TypeRegistry,
    ) -> Result<(), EncodingError> {
        fn rigid_index_name(var: TyVar) -> String {
            format!("\0mesh_layout_index_{}", var.0)
        }

        fn rigid_indices(ty: &Ty) -> Ty {
            match ty {
                Ty::Nat(index) => Ty::Nat(index.map_leaf(|leaf| match leaf {
                    NatIndex::Variable(var) => NatIndex::Parameter(rigid_index_name(*var)),
                    _ => leaf.clone(),
                })),
                _ => ty.map_parts(rigid_indices),
            }
        }

        let mut expressions: Vec<_> = entry.expression_types.iter().collect();
        expressions.sort_unstable_by_key(|(range, _)| (range.start(), range.end()));
        let mut pending: Vec<_> = std::iter::once((&entry.signature, 0))
            .chain(expressions.into_iter().map(|(_, ty)| (ty, 0)))
            .collect();
        let mut instances = Vec::new();
        let mut parameters = BTreeSet::new();
        while let Some((ty, depth)) = pending.pop() {
            if depth >= 64 {
                return Err("native indexed layout source type depth exceeded".into());
            }
            if ty.con().is_some_and(|con| {
                con.indexed_origin.is_some()
                    || registry.sum_type_defs.contains_key(&con.name)
                    || registry.struct_defs.contains_key(&con.name)
            }) {
                instances.push(ty);
            }
            match ty {
                Ty::App(_, args) => pending.extend(args.iter().map(|arg| (arg, depth + 1))),
                Ty::Nat(index) => {
                    if let NatIndex::Variable(var) = index_parts(index)?.0 {
                        parameters.insert(rigid_index_name(*var));
                    }
                }
                _ => pending.extend(ty.parts().map(|part| (part, depth + 1))),
            }
        }
        let parameters: Vec<_> = parameters.into_iter().collect();
        let mut scope = TypeScope::new(self, registry);
        scope.parameters = &parameters;
        scope.rigid_kinds = vec![ParameterKind::Nat; parameters.len()];
        for ty in instances {
            scope
                .source(&rigid_indices(ty))?
                .require_mir(&resolve_type(ty, registry))?;
        }
        Ok(())
    }

    /// Expose each feasible constructor's spelling, Lean symbol, and explicit fields.
    pub(crate) fn nominal_constructors(
        &mut self,
        ty: &Ty,
        registry: &TypeRegistry,
    ) -> Result<Vec<(String, String, Vec<String>)>, EncodingError> {
        let ModelType::Nominal(ty) = TypeScope::new(self, registry).source(ty)? else {
            return Err("kernel model constructors require a checked nominal type".into());
        };
        Ok(self.nominals[&ty.key]
            .constructors
            .iter()
            .enumerate()
            .filter_map(|(index, ctor)| {
                let fields = ctor.values.as_ref()?.iter().map(ModelType::lean).collect();
                Some((
                    ctor.name.clone(),
                    format!("{}.ctor{index}", ty.family),
                    fields,
                ))
            })
            .collect())
    }

    /// Sum constructors use their checked spelling. A struct uses "mk".
    #[cfg(test)]
    pub(crate) fn constructor(
        &mut self,
        ty: &Ty,
        registry: &TypeRegistry,
        variant: &str,
    ) -> Result<String, EncodingError> {
        let ModelType::Nominal(ty) = TypeScope::new(self, registry).source(ty)? else {
            return Err("kernel model constructor requires a checked nominal type".into());
        };
        let (symbol, _) = self.nominals[&ty.key].constructor(variant)?;
        Ok(symbol)
    }

    /// Return the Lean family and its constructors in checked declaration order.
    pub(crate) fn nominal_family(
        &mut self,
        name: &str,
        registry: &TypeRegistry,
    ) -> Result<(String, Vec<(String, String)>), EncodingError> {
        let family = TypeScope::new(self, registry).load_family(name)?;
        let constructors = self.families[name]
            .constructors
            .iter()
            .enumerate()
            .map(|(index, ctor)| (ctor.name.clone(), format!("{family}.ctor{index}")))
            .collect();
        let family = if self.families[name].reorders_arguments() {
            format!("{family}_source")
        } else {
            family
        };
        Ok((family, constructors))
    }

    /// Encode a raw lowering at the caller-supplied rigid Type and Nat parameters.
    /// Lean binders follow the given parameter order. The caller checks the
    /// template's correspondence with each native specialization.
    pub(crate) fn parametric_definition(
        &mut self,
        function: &MirFunction,
        entry: &LoweredSignature,
        registry: &TypeRegistry,
        parameters: &[String],
        emitted: &str,
    ) -> Result<String, EncodingError> {
        self.encode_parametric(function, entry, registry, parameters, emitted, None)
    }

    /// The caller must establish that each alias is a checked variant of this
    /// same declaration and has the corresponding executable recursive body.
    pub(crate) fn parametric_definition_with_recursive_aliases(
        &mut self,
        function: &MirFunction,
        entry: &LoweredSignature,
        registry: &TypeRegistry,
        parameters: &[String],
        emitted: &str,
        recursive_aliases: &HashSet<String>,
    ) -> Result<String, EncodingError> {
        if recursive_aliases.is_empty() {
            return self.parametric_definition(function, entry, registry, parameters, emitted);
        }
        self.encode_parametric(
            function,
            entry,
            registry,
            parameters,
            emitted,
            Some(recursive_aliases),
        )
    }

    fn encode_parametric(
        &mut self,
        function: &MirFunction,
        entry: &LoweredSignature,
        registry: &TypeRegistry,
        parameters: &[String],
        emitted: &str,
        recursive_aliases: Option<&HashSet<String>>,
    ) -> Result<String, EncodingError> {
        (|| {
            if entry.symbol != function.name {
                return Err("conflicting checked binding for the rigid MIR template".into());
            }
            if parameters.is_empty()
                || parameters.iter().any(|name| !name.starts_with('\0'))
                || parameters.iter().collect::<BTreeSet<_>>().len() != parameters.len()
            {
                return Err("invalid rigid kernel model parameters".into());
            }
            let mut scope = TypeScope::new(self, registry);
            scope.parameters = parameters;
            scope.rigid_kinds = rigid_parameter_kinds(&entry.signature, parameters)?;
            scope.recursive_aliases = recursive_aliases;
            let (params, result) = scope.signature(&entry.signature)?;
            Encoder::define(
                function,
                emitted,
                &params,
                &result,
                &mut scope,
                0,
                DefinitionScope::Parametric,
                Some(entry),
            )
        })()
        .map_err(|mut error: EncodingError| {
            error.location = error.location.or(function.body.source_span());
            error
        })
    }

    #[cfg(test)]
    pub(crate) fn definition(
        &mut self,
        function: &MirFunction,
        signature: &Ty,
        registry: &TypeRegistry,
        symbol: &str,
    ) -> Result<String, EncodingError> {
        self.checked_definition(
            function,
            signature,
            registry,
            symbol,
            DefinitionScope::Project,
        )
        .map(|(definition, _)| definition)
        .map_err(|mut error| {
            error.location = error.location.or(function.body.source_span());
            error
        })
    }

    fn checked_definition(
        &mut self,
        function: &MirFunction,
        signature: &Ty,
        registry: &TypeRegistry,
        symbol: &str,
        definition_scope: DefinitionScope,
    ) -> Result<(String, Vec<ModelType>), EncodingError> {
        let lowering = if definition_scope == DefinitionScope::Project {
            self.callees
                .get(function.name.as_str())
                .map(|callee| {
                    if !std::ptr::eq(callee.function, function) || callee.signature != signature {
                        return Err(EncodingError::from(format!(
                            "conflicting checked binding for `{}`",
                            function.name
                        )));
                    }
                    callee.checked_lowering()
                })
                .transpose()?
                .flatten()
        } else {
            None
        };
        let (params, result) = TypeScope::new(self, registry).signature(signature)?;
        let definition = self.encode_function(
            function,
            registry,
            symbol,
            &params,
            &result,
            0,
            definition_scope,
            lowering,
        )?;
        Ok((definition, params))
    }

    fn encode_function(
        &mut self,
        function: &MirFunction,
        registry: &TypeRegistry,
        symbol: &str,
        params: &[ModelType],
        result: &ModelType,
        depth: usize,
        definition_scope: DefinitionScope,
        lowering: Option<&LoweredSignature>,
    ) -> Result<String, EncodingError> {
        self.active_functions.push(function.name.clone());
        let mut scope = TypeScope::new(self, registry);
        let definition = Encoder::define(
            function,
            symbol,
            params,
            result,
            &mut scope,
            depth,
            definition_scope,
            lowering,
        );
        self.active_functions.pop();
        definition
    }

    fn helper(&mut self, name: &str, depth: usize) -> Result<Helper, EncodingError> {
        let Some((&symbol, &callee)) = self.callees.get_key_value(name) else {
            return Err(format!("unsupported MIR call to `{name}`: no unique checked body").into());
        };
        if callee.function.name != symbol {
            return Err(format!("MIR symbol disagrees with checked helper `{name}`").into());
        }
        if let Some(start) = self
            .active_functions
            .iter()
            .position(|active| active == name)
        {
            let cycle = self.active_functions[start..].join(" -> ");
            return Err(format!("cyclic kernel model helper calls: {cycle} -> {name}").into());
        }
        if let Some(helper) = self.helpers.get(symbol) {
            return Ok(helper.clone());
        }

        let helper = (|| {
            let (params, result) =
                TypeScope::new(self, callee.registry).signature(callee.signature)?;
            let emitted = format!("mesh_helper_{}", self.next_helper);
            self.next_helper += 1;
            let definition = self.encode_function(
                callee.function,
                callee.registry,
                &emitted,
                &params,
                &result,
                depth,
                DefinitionScope::Project,
                callee.checked_lowering()?,
            )?;
            self.helper_definitions.push_str(&definition);
            Ok(Helper {
                symbol: emitted,
                params,
                result,
            })
        })()
        .map_err(|mut error: EncodingError| {
            error.reason = format!("helper `{name}`: {}", error.reason);
            error.location = error.location.or(callee.function.body.source_span());
            error
        })?;
        self.helpers.insert(symbol, helper.clone());
        Ok(helper)
    }

    pub(super) fn translate(
        &mut self,
        actual: &MirFunction,
        actual_registry: &TypeRegistry,
        expected: &MirFunction,
        expected_registry: &TypeRegistry,
        signature: &Ty,
    ) -> Result<KernelModel, EncodingError> {
        let mut definitions = String::new();
        let mut parameter_types = Vec::new();
        for (function, registry, symbol, label, definition_scope) in [
            (
                actual,
                actual_registry,
                "mesh_actual",
                "implementation",
                DefinitionScope::Project,
            ),
            (
                expected,
                expected_registry,
                "mesh_expected",
                "expected expression",
                DefinitionScope::StandaloneLaw,
            ),
        ] {
            let (definition, params) = self
                .checked_definition(function, signature, registry, symbol, definition_scope)
                .map_err(|mut error| {
                    error.reason = format!("{label}: {}", error.reason);
                    error.location = error.location.or(function.body.source_span());
                    error
                })?;
            parameter_types = params.iter().map(ModelType::lean).collect();
            definitions.push_str(&definition);
        }
        Ok(KernelModel {
            definitions: format!(
                "{}{}{definitions}",
                self.prelude(),
                self.helper_definitions()
            ),
            parameter_types,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NominalKind {
    Struct,
    Sum,
}

#[derive(Clone, PartialEq, Eq)]
struct NominalType {
    head: TyCon,
    arguments: Vec<Ty>,
    lean_arguments: Vec<String>,
    abi_name: String,
    kind: NominalKind,
    family: String,
    key: String,
}

impl NominalType {
    fn source(&self) -> Ty {
        let head = Ty::Con(self.head.clone());
        if self.arguments.is_empty() {
            head
        } else {
            Ty::App(Box::new(head), self.arguments.clone())
        }
    }

    fn lean(&self) -> String {
        let mut ty = self.family.clone();
        if self.lean_arguments.is_empty() {
            return ty;
        }
        for argument in &self.lean_arguments {
            write!(ty, " ({argument})").unwrap();
        }
        format!("({ty})")
    }

    fn mir(&self) -> MirType {
        match self.kind {
            NominalKind::Struct => MirType::Struct(self.abi_name.clone()),
            NominalKind::Sum => MirType::SumType(self.abi_name.clone()),
        }
    }
}

// Encode checked constructor identities and argument trees, ignoring
// display prefixes just as Ty equality does.
fn nominal_symbol(
    name: &str,
    origin: Option<&str>,
    args: &[Ty],
    parameters: &[String],
    depth: usize,
) -> Result<String, EncodingError> {
    fn write_name(output: &mut String, name: &str) {
        for byte in name.bytes() {
            write!(output, "{byte:02x}").unwrap();
        }
    }
    fn write_type(
        output: &mut String,
        ty: &Ty,
        parameters: &[String],
        depth: usize,
    ) -> Result<(), EncodingError> {
        if depth >= 64 {
            return Err("kernel model source type depth exceeded".into());
        }
        match ty {
            Ty::Con(con) => {
                if let Some(origin) = &con.indexed_origin {
                    output.push('i');
                    write_name(output, origin);
                    output.push('_');
                } else {
                    output.push('c');
                }
                write_name(output, &con.name);
                if let Some(index) = parameters.iter().position(|name| name == &con.name) {
                    write!(output, "p{index}").unwrap();
                }
                output.push('_');
            }
            Ty::Nat(index) => match IndexForm::read(index)? {
                IndexForm::Ground(value) => write!(output, "n{value}_").unwrap(),
                IndexForm::Rigid(name, successors) => {
                    write!(output, "s{successors}p").unwrap();
                    write_name(output, name);
                    if let Some(position) = parameters.iter().position(|param| param == name) {
                        write!(output, "p{position}").unwrap();
                    }
                    output.push('_');
                }
            },
            Ty::App(head, args) if matches!(head.as_ref(), Ty::Con(_)) => {
                write!(output, "a{}_", args.len()).unwrap();
                write_type(output, head, parameters, depth + 1)?;
                for arg in args {
                    write_type(output, arg, parameters, depth + 1)?;
                }
            }
            Ty::Tuple(fields) => {
                write!(output, "t{}_", fields.len()).unwrap();
                for field in fields {
                    write_type(output, field, parameters, depth + 1)?;
                }
            }
            _ => return Err(format!("unsupported kernel model type argument {ty}").into()),
        }
        Ok(())
    }

    let mut output = String::from("_mesh_nominal_");
    write_name(&mut output, name);
    if let Some(origin) = origin {
        output.push_str("_i");
        write_name(&mut output, origin);
    }
    if !args.is_empty() {
        output.push('_');
        for arg in args {
            write_type(&mut output, arg, parameters, depth + 1)?;
        }
    }
    Ok(output)
}

fn index_parts(mut index: &NatIndex) -> Result<(&NatIndex, u128), EncodingError> {
    let mut successors = 0;
    while let NatIndex::Succ(inner) = index {
        successors += 1;
        if successors >= 64 {
            return Err("kernel model Nat index depth exceeded".into());
        }
        index = inner;
    }
    Ok((index, successors))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParameterKind {
    Type,
    Nat,
}

fn rigid_parameter_kinds(
    signature: &Ty,
    parameters: &[String],
) -> Result<Vec<ParameterKind>, EncodingError> {
    fn record(
        ty: &Ty,
        parameters: &[String],
        kinds: &mut [Option<ParameterKind>],
        depth: usize,
    ) -> Result<(), EncodingError> {
        if depth >= 64 {
            return Err("kernel model source type depth exceeded".into());
        }
        let occurrence = match ty {
            Ty::Con(con) if con.indexed_origin.is_none() && con.name.starts_with('\0') => {
                Some((&con.name, ParameterKind::Type))
            }
            Ty::Nat(index) => match index_parts(index)?.0 {
                NatIndex::Literal(_) => None,
                NatIndex::Parameter(name) if name.starts_with('\0') => {
                    Some((name, ParameterKind::Nat))
                }
                _ => return Err("open Nat index in checked rigid function signature".into()),
            },
            _ => None,
        };
        if let Some((name, kind)) = occurrence {
            let position = parameters
                .iter()
                .position(|parameter| parameter == name)
                .ok_or("undeclared rigid kernel model parameter")?;
            if kinds[position].is_some_and(|prior| prior != kind) {
                return Err("rigid kernel model parameter occurs at both Type and Nat kind".into());
            }
            kinds[position] = Some(kind);
        }
        for part in ty.parts() {
            record(part, parameters, kinds, depth + 1)?;
        }
        Ok(())
    }

    let mut kinds = vec![None; parameters.len()];
    record(signature, parameters, &mut kinds, 0)?;
    kinds
        .into_iter()
        .map(|kind| kind.ok_or_else(|| "unused rigid kernel model parameter".into()))
        .collect()
}

// Fold successors into a ground value, or retain the exact rigid identity and
// its number of successors. u128 leaves room beyond the largest source literal.
#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexForm<'a> {
    Ground(u128),
    Rigid(&'a str, u128),
}

impl<'a> IndexForm<'a> {
    fn read(index: &'a NatIndex) -> Result<Self, EncodingError> {
        let (leaf, successors) = index_parts(index)?;
        match leaf {
            NatIndex::Literal(n) => Ok(Self::Ground(u128::from(*n) + successors)),
            NatIndex::Parameter(name) => Ok(Self::Rigid(name, successors)),
            _ => Err("open Nat index in kernel model type".into()),
        }
    }

    fn lower_bound(self) -> u128 {
        match self {
            Self::Ground(value) | Self::Rigid(_, value) => value,
        }
    }

    fn disjoint(self, other: Self) -> bool {
        match (self, other) {
            (Self::Ground(a), Self::Ground(b)) => a != b,
            (Self::Rigid(a, da), Self::Rigid(b, db)) => a == b && da != db,
            (Self::Ground(value), Self::Rigid(_, successors))
            | (Self::Rigid(_, successors), Self::Ground(value)) => value < successors,
        }
    }

    fn subtract(self, successors: u128) -> Option<Self> {
        let remaining = self.lower_bound().checked_sub(successors)?;
        Some(match self {
            Self::Ground(_) => Self::Ground(remaining),
            Self::Rigid(name, _) => Self::Rigid(name, remaining),
        })
    }

    fn into_index(self) -> NatIndex {
        let (mut index, remaining) = match self {
            Self::Ground(value) => {
                let literal = value.min(u128::from(u64::MAX));
                (NatIndex::Literal(literal as u64), value - literal)
            }
            Self::Rigid(name, successors) => (NatIndex::Parameter(name.into()), successors),
        };
        for _ in 0..remaining {
            index = NatIndex::Succ(Box::new(index));
        }
        index
    }
}

#[derive(PartialEq, Eq)]
struct CheckedNominal {
    kind: NominalKind,
    parameters: Vec<String>,
    origin: Option<String>,
    index_positions: Vec<usize>,
    constructors: Vec<CheckedConstructor>,
}

#[derive(PartialEq, Eq)]
struct CheckedConstructor {
    name: String,
    fields: Vec<(Option<String>, Ty)>,
    indexed: Option<IndexedConstructor>,
}

#[derive(PartialEq, Eq)]
struct IndexedConstructor {
    parameters: Vec<String>,
    result: Ty,
}

enum ConstructorIndices {
    Exact(BTreeMap<String, Ty>),
    Impossible,
    RefinementRequired,
}

impl IndexedConstructor {
    fn bindings(&self, args: &[Ty]) -> Result<ConstructorIndices, EncodingError> {
        let Ty::App(_, result) = &self.result else {
            return Err("invalid checked indexed constructor result".into());
        };
        let mut bindings = BTreeMap::new();
        let mut needs_refinement = false;
        for (wanted, actual) in result.iter().zip(args) {
            let (Ty::Nat(wanted), Ty::Nat(actual)) = (wanted, actual) else {
                continue;
            };
            let (leaf, successors) = index_parts(wanted)?;
            let actual = IndexForm::read(actual)?;
            match leaf {
                NatIndex::Literal(n) => {
                    let wanted = IndexForm::Ground(u128::from(*n) + successors);
                    if wanted.disjoint(actual) {
                        return Ok(ConstructorIndices::Impossible);
                    }
                    needs_refinement |= wanted != actual;
                }
                NatIndex::Parameter(name) if self.parameters.contains(name) => {
                    let Some(value) = actual.subtract(successors) else {
                        if matches!(actual, IndexForm::Ground(_)) {
                            return Ok(ConstructorIndices::Impossible);
                        }
                        needs_refinement = true;
                        continue;
                    };
                    if let Some(prior) = bindings.get(name) {
                        if value.disjoint(*prior) {
                            return Ok(ConstructorIndices::Impossible);
                        }
                        needs_refinement |= *prior != value;
                    } else {
                        bindings.insert(name.clone(), value);
                    }
                }
                _ => return Err("unmodeled indexed constructor result".into()),
            }
        }
        if needs_refinement {
            return Ok(ConstructorIndices::RefinementRequired);
        }
        if bindings.len() != self.parameters.len() {
            return Err("missing exact binding for a constructor's Nat index".into());
        }
        Ok(ConstructorIndices::Exact(
            bindings
                .into_iter()
                .map(|(name, value)| (name, Ty::Nat(value.into_index())))
                .collect(),
        ))
    }
}

impl CheckedNominal {
    fn reorders_arguments(&self) -> bool {
        let types = self.parameters.len() - self.index_positions.len();
        self.index_positions
            .first()
            .is_some_and(|&first| first < types)
    }

    fn read(name: &str, registry: &TypeRegistry) -> Result<Self, EncodingError> {
        let source = Ty::Con(TyCon::new(name));
        if registry.is_resource_type(&source) || registry.is_plaintext_type(&source) {
            return Err(format!(
                "kernel model nominal type `{name}` is resource-bearing or plaintext"
            )
            .into());
        }
        let sum = registry.sum_type_defs.get(name);
        let indexed = sum.and_then(|def| def.indexed.as_ref());
        let (kind, parameters, constructors) = match (registry.struct_defs.get(name), sum) {
            (Some(def), None) if def.name == name => (
                NominalKind::Struct,
                &def.generic_params,
                vec![CheckedConstructor {
                    name: "mk".into(),
                    fields: def
                        .fields
                        .iter()
                        .map(|(name, ty)| (Some(name.clone()), ty.clone()))
                        .collect(),
                    indexed: None,
                }],
            ),
            (None, Some(def)) if def.name == name => (
                NominalKind::Sum,
                &def.generic_params,
                def.variants
                    .iter()
                    .map(|variant| CheckedConstructor {
                        name: variant.name.clone(),
                        fields: variant
                            .fields
                            .iter()
                            .map(|field| match field {
                                VariantFieldInfo::Positional(ty) => (None, ty.clone()),
                                VariantFieldInfo::Named(name, ty) => {
                                    (Some(name.clone()), ty.clone())
                                }
                            })
                            .collect(),
                        indexed: variant.indexed.as_ref().map(|variant| IndexedConstructor {
                            parameters: variant.index_params.clone(),
                            result: variant.result.clone(),
                        }),
                    })
                    .collect(),
            ),
            _ => return Err(format!("missing or ambiguous checked type `{name}`").into()),
        };
        if parameters.iter().collect::<BTreeSet<_>>().len() != parameters.len() {
            return Err(format!("duplicate checked type parameter in `{name}`").into());
        }
        if constructors.is_empty() || constructors.len() > 256 {
            return Err(format!("invalid number of native constructors for `{name}`").into());
        }
        let origin = indexed.map(|info| info.origin.clone());
        let index_positions = indexed.map_or_else(Vec::new, |info| info.index_positions.clone());
        if origin.is_some()
            && (index_positions.is_empty()
                || index_positions.windows(2).any(|pair| pair[0] >= pair[1])
                || index_positions
                    .iter()
                    .any(|&position| position >= parameters.len()))
        {
            return Err(format!("invalid checked index positions for `{name}`").into());
        }
        let mut names = BTreeSet::new();
        for ctor in &constructors {
            let mut fields = BTreeSet::new();
            if !names.insert(&ctor.name)
                || ctor
                    .fields
                    .iter()
                    .filter_map(|(name, _)| name.as_ref())
                    .any(|name| !fields.insert(name))
            {
                return Err(format!("duplicate checked constructor or field in `{name}`").into());
            }
            match (&origin, &ctor.indexed) {
                (None, None) => {}
                (Some(origin), Some(indexed)) => {
                    let Ty::App(head, args) = &indexed.result else {
                        return Err(
                            format!("invalid checked constructor result for `{name}`").into()
                        );
                    };
                    if !matches!(head.as_ref(), Ty::Con(con) if con.name == name && con.indexed_origin.as_deref() == Some(origin))
                        || args.len() != parameters.len()
                        || indexed.parameters.iter().collect::<BTreeSet<_>>().len()
                            != indexed.parameters.len()
                        || args.iter().zip(parameters).enumerate().any(
                            |(position, (arg, parameter))| {
                                if index_positions.contains(&position) {
                                    !matches!(arg, Ty::Nat(_))
                                } else {
                                    *arg != Ty::Con(TyCon::new(parameter))
                                }
                            },
                        )
                    {
                        return Err(
                            format!("invalid checked constructor result for `{name}`").into()
                        );
                    }
                }
                _ => return Err(format!("inconsistent checked index metadata for `{name}`").into()),
            }
        }
        Ok(Self {
            kind,
            parameters: parameters.clone(),
            origin,
            index_positions,
            constructors,
        })
    }
}

#[derive(PartialEq, Eq)]
struct NativeConstructor {
    name: String,
    fields: Vec<(Option<String>, MirType)>,
    // An infeasible constructor still occupies its declared native tag and storage.
    values: Option<Vec<ModelType>>,
    needs_refinement: bool,
}

#[derive(PartialEq, Eq)]
struct NominalDef {
    ty: NominalType,
    constructors: Vec<NativeConstructor>,
}

struct NominalField {
    ty: ModelType,
    storage: MirType,
}

impl NominalDef {
    fn constructor(
        &self,
        name: &str,
    ) -> Result<(String, Vec<(Option<String>, NominalField)>), EncodingError> {
        let (index, ctor) = self
            .constructors
            .iter()
            .enumerate()
            .find(|(_, ctor)| ctor.name == name)
            .ok_or_else(|| format!("unknown checked constructor `{}.{name}`", self.ty.head.name))?;
        let values = ctor.values.as_ref().ok_or_else(|| {
            if ctor.needs_refinement {
                return format!(
                    "constructor `{}.{name}` needs a Nat index refinement for {}",
                    self.ty.head.name,
                    self.ty.lean()
                );
            }
            format!(
                "constructor `{}.{name}` cannot inhabit {}",
                self.ty.head.name,
                self.ty.lean()
            )
        })?;
        let fields = ctor
            .fields
            .iter()
            .zip(values)
            .map(|((name, storage), ty)| {
                (
                    name.clone(),
                    NominalField {
                        ty: ty.clone(),
                        storage: storage.clone(),
                    },
                )
            })
            .collect();
        Ok((format!("{}.ctor{index}", self.ty.family), fields))
    }

    fn has_pointer_layout(&self) -> bool {
        // Native sums use a pointer-aligned record exactly when all
        // nonempty variants have a single pointer slot.
        self.constructors.iter().any(|ctor| !ctor.fields.is_empty())
            && self
                .constructors
                .iter()
                .all(|ctor| match ctor.fields.as_slice() {
                    [] => true,
                    [(_, storage)] => matches!(storage, MirType::Ptr | MirType::Struct(_)),
                    _ => false,
                })
    }

    fn require_layout(&self, modules: &[MirModule]) -> Result<(), EncodingError> {
        let name = match self.ty.kind {
            NominalKind::Struct => &self.ty.abi_name,
            NominalKind::Sum => &self.ty.head.name,
        };
        let mut found = false;
        for module in modules {
            let mut structs = module.structs.iter().filter(|def| &def.name == name);
            let mut sums = module.sum_types.iter().filter(|def| &def.name == name);
            let agrees = match (self.ty.kind, structs.next(), sums.next()) {
                (_, None, None) => continue,
                (NominalKind::Struct, Some(def), None) => {
                    let ctor = &self.constructors[0];
                    def.fields.len() == ctor.fields.len()
                        && def.fields.iter().zip(&ctor.fields).all(
                            |((field, storage), (checked_name, checked))| {
                                Some(field) == checked_name.as_ref() && storage == checked
                            },
                        )
                }
                (NominalKind::Sum, None, Some(def)) => {
                    def.variants.len() == self.constructors.len()
                        && def.variants.iter().zip(&self.constructors).enumerate().all(
                            |(tag, (variant, ctor))| {
                                variant.name == ctor.name
                                    && usize::from(variant.tag) == tag
                                    && variant
                                        .fields
                                        .iter()
                                        .eq(ctor.fields.iter().map(|(_, storage)| storage))
                            },
                        )
                }
                _ => false,
            };
            if !agrees || structs.next().is_some() || sums.next().is_some() {
                return Err(format!(
                    "checked declaration and native MIR layout disagree for `{name}`"
                )
                .into());
            }
            found = true;
        }
        if !found {
            return Err(format!("missing exact native MIR layout for `{name}`").into());
        }
        Ok(())
    }
}

/// Validation is scoped to the checked registry of a single translation.
/// A shared Lean declaration never stands in for validation in a new scope.
struct TypeScope<'session, 'project, 'registry> {
    session: &'session mut ModelSession<'project>,
    registry: &'registry TypeRegistry,
    parameters: &'registry [String],
    rigid_kinds: Vec<ParameterKind>,
    recursive_aliases: Option<&'registry HashSet<String>>,
    index_parameters: &'registry [String],
    arm_binder: Option<(String, String)>,
    // Rendering a checked family does not require a layout for abstract values.
    family_owner: Option<&'registry str>,
    validated: BTreeMap<String, NominalType>,
    active: Vec<NominalType>,
}

impl<'session, 'project, 'registry> TypeScope<'session, 'project, 'registry> {
    fn new(
        session: &'session mut ModelSession<'project>,
        registry: &'registry TypeRegistry,
    ) -> Self {
        Self {
            session,
            registry,
            parameters: &[],
            rigid_kinds: Vec::new(),
            recursive_aliases: None,
            index_parameters: &[],
            arm_binder: None,
            family_owner: None,
            validated: BTreeMap::new(),
            active: Vec::new(),
        }
    }

    fn rigid_binder(&self, position: usize) -> (String, &'static str) {
        match self.rigid_kinds[position] {
            ParameterKind::Type => (parameter_symbol(position), "Type"),
            ParameterKind::Nat => (index_symbol(position), "Nat"),
        }
    }

    fn substitute(
        &mut self,
        ty: &ModelType,
        name: &str,
        replacement: &Ty,
    ) -> Result<ModelType, EncodingError> {
        match ty {
            ModelType::Nominal(nominal) => {
                let source = nominal.source();
                let substituted = source.replace_cons(&mut |con| {
                    (con.indexed_origin.is_none() && con.name == name).then(|| replacement.clone())
                });
                if source == substituted {
                    return Ok(ty.clone());
                }
                self.source(&substituted)
            }
            ModelType::List(element) => Ok(ModelType::List(Box::new(self.substitute(
                element,
                name,
                replacement,
            )?))),
            ModelType::Tuple(fields) => Ok(ModelType::Tuple(
                fields
                    .iter()
                    .map(|ty| self.substitute(ty, name, replacement))
                    .collect::<Result<_, _>>()?,
            )),
            _ => Ok(ty.clone()),
        }
    }

    fn signature(&mut self, signature: &Ty) -> Result<(Vec<ModelType>, ModelType), EncodingError> {
        let Ty::Fun(params, result) = signature else {
            return Err("expected a checked function signature for the kernel model".into());
        };
        let params = params
            .iter()
            .map(|ty| self.source(ty))
            .collect::<Result<_, _>>()?;
        Ok((params, self.source(result)?))
    }

    fn source(&mut self, ty: &Ty) -> Result<ModelType, EncodingError> {
        self.source_at(ty, 0)
    }

    fn source_at(&mut self, ty: &Ty, depth: usize) -> Result<ModelType, EncodingError> {
        if depth >= 64 {
            return Err("kernel model source type depth exceeded".into());
        }
        // A checked project declaration may share a builtin's bare name.
        // Its appearance must not acquire the builtin's Lean meaning.
        let builtin = |name: &str| {
            !self.registry.is_resource_name(name)
                && !self.registry.plaintext_types.contains(name)
                && !self.registry.struct_defs.contains_key(name)
                && !self.registry.sum_type_defs.contains_key(name)
                && !self
                    .session
                    .declarations
                    .iter()
                    .any(|decl| decl.name == name)
        };
        match ty {
            Ty::Con(con)
                if con.indexed_origin.is_none()
                    && (self.parameters.contains(&con.name) || con.name.starts_with('\0')) =>
            {
                let index = self
                    .parameters
                    .iter()
                    .position(|name| name == &con.name)
                    .ok_or("undeclared rigid kernel model type parameter")?;
                if self.rigid_kinds.get(index) == Some(&ParameterKind::Nat) {
                    return Err("rigid Nat parameter used as a kernel model type".into());
                }
                let abi = MirType::Struct(con.name.clone());
                if self.family_owner.is_none() && resolve_type(ty, self.registry) != abi {
                    return Err("rigid kernel model type has a runtime ABI override".into());
                }
                Ok(ModelType::Parameter { index, abi })
            }
            Ty::Con(con)
                if con.indexed_origin.is_none() && con.name == "Bool" && builtin("Bool") =>
            {
                Ok(ModelType::Bool)
            }
            Ty::Con(con) if con.indexed_origin.is_none() && con.name == "Int" && builtin("Int") => {
                Ok(ModelType::Int)
            }
            Ty::Con(con)
                if con.indexed_origin.is_none()
                    && matches!(con.name.as_str(), "Unit" | "()")
                    && builtin(&con.name) =>
            {
                Ok(ModelType::Unit)
            }
            Ty::Tuple(elements) if elements.is_empty() => Ok(ModelType::Unit),
            Ty::Tuple(elements) if elements.len() >= 2 => {
                let elements = elements
                    .iter()
                    .map(|ty| self.source_at(ty, depth + 1))
                    .collect::<Result<_, _>>()?;
                Ok(ModelType::Tuple(elements))
            }
            Ty::Con(con) => self
                .load(&con.name, &[], ty, depth + 1)
                .map(ModelType::Nominal),
            Ty::App(head, args) => {
                let Ty::Con(con) = head.as_ref() else {
                    return Err(format!("unsupported kernel model source type {ty}").into());
                };
                if con.indexed_origin.is_none()
                    && (con.name.starts_with('\0') || self.parameters.contains(&con.name))
                {
                    return Err(format!("unsupported kernel model type application {ty}").into());
                }
                if con.indexed_origin.is_none() && !args.is_empty() && builtin(&con.name) {
                    if let ("List", [element]) = (con.name.as_str(), args.as_slice()) {
                        return Ok(ModelType::List(Box::new(
                            self.source_at(element, depth + 1)?,
                        )));
                    }
                    return Err(format!("unsupported kernel model source type {ty}").into());
                }
                self.load(&con.name, args, ty, depth + 1)
                    .map(ModelType::Nominal)
            }
            _ => Err(format!("unsupported kernel model source type {ty}").into()),
        }
    }

    fn checked_nominal(
        &self,
        name: &str,
    ) -> Result<(CheckedNominal, &'project TypeRegistry), EncodingError> {
        let mut owners = self
            .session
            .declarations
            .iter()
            .filter(|decl| decl.name == name);
        let owner = owners.next().ok_or_else(|| {
            format!("missing checked source owner for kernel model type `{name}`")
        })?;
        if owners.next().is_some() {
            return Err(
                format!("ambiguous checked source owners for kernel model type `{name}`").into(),
            );
        }
        let checked = CheckedNominal::read(name, self.registry)?;
        if checked != CheckedNominal::read(name, owner.registry)? {
            return Err(
                format!("nominal type `{name}` disagrees with its checked source owner").into(),
            );
        }
        Ok((checked, owner.registry))
    }

    fn load_family(&mut self, name: &str) -> Result<String, EncodingError> {
        let family = nominal_symbol(name, None, &[], &[], 0)?;
        let (checked, registry) = self.checked_nominal(name)?;
        if let Some(previous) = self.session.families.get(name) {
            if previous != &checked {
                return Err(format!("conflicting checked family for nominal type `{name}`").into());
            }
            return Ok(family);
        }
        if self
            .session
            .active_families
            .iter()
            .any(|active| active == name)
        {
            return Err(
                format!("unsupported mutually recursive kernel model type `{name}`").into(),
            );
        }
        if self.session.active_families.len() >= 64 {
            return Err("kernel model nominal family depth exceeded".into());
        }

        self.session.active_families.push(name.to_owned());
        let mut template = TypeScope::new(self.session, registry);
        let type_parameters: Vec<_> = checked
            .parameters
            .iter()
            .enumerate()
            .filter_map(|(position, name)| {
                (!checked.index_positions.contains(&position)).then_some(name.clone())
            })
            .collect();
        template.parameters = &type_parameters;
        template.family_owner = Some(name);
        let definition = (|| {
            let mut output = format!("inductive {family}");
            let mut result = family.clone();
            for index in 0..type_parameters.len() {
                let parameter = parameter_symbol(index);
                write!(output, " ({parameter} : Type)").unwrap();
                write!(result, " {parameter}").unwrap();
            }
            if !checked.index_positions.is_empty() {
                output.push_str(" : ");
                for _ in &checked.index_positions {
                    output.push_str("Nat → ");
                }
                output.push_str("Type");
            }
            output.push_str(" where\n");
            for (index, ctor) in checked.constructors.iter().enumerate() {
                write!(output, "  | ctor{index} : ").unwrap();
                template.index_parameters = ctor
                    .indexed
                    .as_ref()
                    .map_or(&[], |info| info.parameters.as_slice());
                for index in 0..template.index_parameters.len() {
                    write!(output, "{{{} : Nat}} → ", index_symbol(index)).unwrap();
                }
                for (_, field) in &ctor.fields {
                    write!(output, "{} → ", template.source(field)?.lean()).unwrap();
                }
                if let Some(indexed) = &ctor.indexed {
                    writeln!(output, "{}", template.source(&indexed.result)?.lean()).unwrap();
                } else {
                    writeln!(output, "{result}").unwrap();
                }
            }
            if checked.reorders_arguments() {
                write!(output, "\nabbrev {family}_source").unwrap();
                let mut types = 0;
                let mut indices = 0;
                for position in 0..checked.parameters.len() {
                    if checked.index_positions.contains(&position) {
                        write!(output, " ({} : Nat)", index_symbol(indices)).unwrap();
                        indices += 1;
                    } else {
                        write!(output, " ({} : Type)", parameter_symbol(types)).unwrap();
                        types += 1;
                    }
                }
                write!(output, " : Type := {result}").unwrap();
                for index in 0..indices {
                    write!(output, " {}", index_symbol(index)).unwrap();
                }
                output.push('\n');
            }
            output.push('\n');
            Ok::<_, EncodingError>(output)
        })();
        self.session.active_families.pop();
        self.session.prelude.push_str(&definition?);
        self.session.families.insert(name.to_owned(), checked);
        Ok(family)
    }

    fn render_index(&self, ty: &Ty) -> Result<String, EncodingError> {
        let Ty::Nat(index) = ty else {
            return Err(format!("expected a checked Nat index, found {ty}").into());
        };
        let (leaf, successors) = index_parts(index)?;
        if let NatIndex::Literal(n) = leaf {
            return Ok((u128::from(*n) + successors).to_string());
        }
        if let NatIndex::Parameter(name) = leaf {
            let position = if self.family_owner.is_some() {
                self.index_parameters.iter().position(|param| param == name)
            } else {
                self.parameters
                    .iter()
                    .zip(&self.rigid_kinds)
                    .position(|(param, kind)| param == name && *kind == ParameterKind::Nat)
            };
            let value = position.map(index_symbol).or_else(|| {
                let (binder, symbol) = self.arm_binder.as_ref()?;
                (binder == name).then(|| symbol.clone())
            });
            if let Some(mut value) = value {
                for _ in 0..successors {
                    value = format!("(Nat.succ {value})");
                }
                return Ok(value);
            }
        }
        Err("open Nat index in kernel model type".into())
    }

    fn registries(&self) -> impl Iterator<Item = &TypeRegistry> {
        std::iter::once(self.registry)
            .chain(self.session.declarations.iter().map(|decl| decl.registry))
    }

    fn require_native_name(&self, base: &str, abi: &str) -> Result<(), EncodingError> {
        // A checked exact name can shadow the lowered name of a generic
        // instance. Different generic bases can also produce the same name.
        let generic_base = |name: &str| {
            self.registries().any(|registry| {
                registry
                    .struct_defs
                    .get(name)
                    .is_some_and(|def| !def.generic_params.is_empty())
                    || registry
                        .sum_type_defs
                        .get(name)
                        .is_some_and(|def| !def.generic_params.is_empty())
            })
        };
        let exact_shadow = base != abi
            && (self
                .session
                .declarations
                .iter()
                .any(|decl| decl.name == abi)
                || self.registries().any(|registry| {
                    registry.struct_defs.contains_key(abi)
                        || registry.sum_type_defs.contains_key(abi)
                }));
        if exact_shadow
            || abi.match_indices('_').any(|(separator, _)| {
                let prefix = &abi[..separator];
                prefix != base && generic_base(prefix)
            })
        {
            return Err(
                format!("nominal type `{abi}` collides with a generic native ABI name").into(),
            );
        }
        Ok(())
    }

    fn require_sum_fallback(&self, base: &str, abi: &str) -> Result<(), EncodingError> {
        if base == abi {
            return Ok(());
        }
        // Codegen tries the exact name before falling back to its encoded base.
        self.require_native_name(base, abi)?;
        if sum_type_base(abi) != base
            || self.session.modules.iter().any(|module| {
                module.sum_types.iter().any(|def| def.name == abi)
                    || module.structs.iter().any(|def| def.name == abi)
            })
        {
            return Err(format!(
                "native MIR layout for `{abi}` does not uniquely fall back to `{base}`"
            )
            .into());
        }
        Ok(())
    }

    fn sum_storage(
        &self,
        owner: &NominalType,
        template: &Ty,
        direct_parameter: bool,
        value: Option<&ModelType>,
    ) -> Result<MirType, EncodingError> {
        let value_abi = value.map(ModelType::mir);
        if direct_parameter {
            // The base sum layout lowers a bare type parameter to a pointer.
            // Codegen stores an existing pointer or boxes the concrete scalar
            // or aggregate. The pattern column then recovers its concrete type.
            return match value_abi {
                None
                | Some(
                    MirType::Int
                    | MirType::Bool
                    | MirType::Ptr
                    | MirType::Struct(_)
                    | MirType::SumType(_),
                ) => Ok(MirType::Ptr),
                _ => Err(format!("unsupported boxed kernel model payload {template}").into()),
            };
        }

        let template_abi = match resolve_type(template, self.registry) {
            MirType::Tuple(_) => MirType::Ptr,
            mir => mir,
        };
        if let (MirType::SumType(template_name), Some(head)) = (&template_abi, template.con()) {
            if value.is_none_or(|value| {
                matches!(value, ModelType::Nominal(inner)
                if inner.kind == NominalKind::Sum && inner.head == *head)
            }) {
                // Both names resolve to the same validated native sum record.
                self.require_sum_fallback(&head.name, template_name)?;
                if *head == owner.head {
                    return Ok(MirType::Ptr);
                }
                return Ok(template_abi);
            }
        }
        if value_abi.as_ref().is_none_or(|abi| *abi == template_abi) {
            return Ok(template_abi);
        }
        Err(format!(
            "unsupported native storage for `{}` payload {template} as {}",
            owner.head.name,
            value
                .expect("the concrete payload disagrees with its template ABI")
                .lean()
        )
        .into())
    }

    fn load(
        &mut self,
        name: &str,
        args: &[Ty],
        source: &Ty,
        depth: usize,
    ) -> Result<NominalType, EncodingError> {
        let head = source
            .con()
            .ok_or("expected a checked nominal constructor")?;
        let origin = head.indexed_origin.as_deref().map(String::as_str);
        let key = nominal_symbol(name, origin, args, self.parameters, depth)?;
        if let Some(ty) = self.validated.get(&key) {
            return Ok(ty.clone());
        }
        if let Some(ty) = self.active.last().filter(|ty| ty.key == key) {
            return Ok(ty.clone());
        }
        if self.active.iter().any(|ty| ty.key == key) {
            return Err(
                format!("unsupported mutually recursive kernel model type `{name}`").into(),
            );
        }
        let (checked, _) = self.checked_nominal(name)?;
        if checked.origin.as_deref() != origin {
            return Err(format!(
                "kernel model type `{name}` disagrees with its checked indexed origin"
            )
            .into());
        }
        if origin.is_none() && self.active.iter().any(|ty| ty.head.name == name) {
            return Err(format!(
                "unsupported polymorphically recursive kernel model type `{name}`"
            )
            .into());
        }
        if args.len() != checked.parameters.len() {
            return Err(
                format!("kernel model type `{name}` requires concrete type arguments").into(),
            );
        }
        let (indices, type_arguments): (Vec<_>, Vec<_>) = args
            .iter()
            .enumerate()
            .partition(|(position, _)| checked.index_positions.contains(position));
        let mut lean_arguments = Vec::with_capacity(args.len());
        for (_, arg) in &type_arguments {
            lean_arguments.push(self.source_at(arg, depth)?.lean());
        }
        for (_, index) in indices {
            lean_arguments.push(self.render_index(index)?);
        }
        let family = if self.family_owner == Some(name) {
            if !type_arguments.iter().zip(self.parameters).all(|((_, arg), parameter)| {
                matches!(arg, Ty::Con(con) if con.name == *parameter && con.indexed_origin.is_none())
            }) {
                return Err(format!("unsupported polymorphically recursive kernel model type `{name}`").into());
            }
            nominal_symbol(name, None, &[], &[], depth)?
        } else {
            self.load_family(name)?
        };
        let ty = NominalType {
            head: head.clone(),
            arguments: args
                .iter()
                .map(|arg| arg.replace_cons(&mut |_| None))
                .collect(),
            lean_arguments,
            abi_name: if self.family_owner.is_some() {
                name.to_owned()
            } else {
                mangle_type_name(name, args, self.registry)
            },
            kind: checked.kind,
            family,
            key,
        };
        if self.family_owner.is_some() {
            return Ok(ty);
        }
        if origin.is_some() && self.active.len() + self.validated.len() >= 256 {
            return Err("kernel model nominal instance budget exhausted".into());
        }
        if ty.mir() != resolve_type(source, self.registry) {
            return Err(
                format!("kernel model nominal type `{name}` has a runtime ABI override").into(),
            );
        }
        if ty.kind == NominalKind::Sum && !args.is_empty() {
            self.require_sum_fallback(name, &ty.abi_name)?;
        } else {
            self.require_native_name(name, &ty.abi_name)?;
        }

        let substitutions: BTreeMap<_, _> = type_arguments
            .into_iter()
            .map(|(position, arg)| (&checked.parameters[position], arg))
            .collect();
        self.active.push(ty.clone());
        let definition = (|| {
            let mut constructors = Vec::new();
            for ctor in checked.constructors {
                let binding = match &ctor.indexed {
                    Some(indexed) => indexed.bindings(args)?,
                    None => ConstructorIndices::Exact(BTreeMap::new()),
                };
                let indices = match &binding {
                    ConstructorIndices::Exact(indices) => Some(indices),
                    _ => None,
                };
                let mut fields = Vec::new();
                let mut values = Vec::new();
                for (field, template) in ctor.fields {
                    let value = if let Some(indices) = indices {
                        let instantiated = template.replace_cons(&mut |con| {
                            if con.indexed_origin.is_some() {
                                return None;
                            }
                            substitutions
                                .get(&con.name)
                                .copied()
                                .or_else(|| indices.get(&con.name))
                                .cloned()
                        });
                        Some(self.source_at(&instantiated, depth)?)
                    } else {
                        None
                    };
                    let storage = match ty.kind {
                        NominalKind::Struct => {
                            let value = value.as_ref().expect("struct constructors have no Nat indices");
                            if matches!(value, ModelType::Nominal(inner) if *inner == ty) {
                                return Err(format!("native struct `{name}` holds itself by value").into());
                            }
                            value.mir()
                        }
                        NominalKind::Sum => self.sum_storage(
                            &ty,
                            &template,
                            matches!(&template, Ty::Con(con) if con.indexed_origin.is_none() && substitutions.contains_key(&con.name)),
                            value.as_ref(),
                        )?,
                    };
                    fields.push((field, storage));
                    if let Some(value) = value {
                        values.push(value);
                    }
                }
                constructors.push(NativeConstructor {
                    name: ctor.name,
                    fields,
                    values: indices.map(|_| values),
                    needs_refinement: matches!(binding, ConstructorIndices::RefinementRequired),
                });
            }
            Ok::<_, EncodingError>(NominalDef {
                ty: ty.clone(),
                constructors,
            })
        })();
        self.active.pop();
        let definition = definition?;
        match self.session.nominals.get(&ty.key) {
            Some(previous) if previous != &definition => {
                return Err(
                    format!("conflicting checked binding for nominal type {source}").into(),
                );
            }
            Some(_) => {}
            None => {
                definition.require_layout(self.session.modules)?;
                self.session.nominals.insert(ty.key.clone(), definition);
            }
        }
        self.validated.insert(ty.key.clone(), ty.clone());
        Ok(ty)
    }

    fn construction_type(
        &mut self,
        mir: &MirType,
        checked: Option<&ModelType>,
    ) -> Result<NominalType, EncodingError> {
        match checked {
            Some(ModelType::Nominal(ty)) if ty.mir() == *mir => {
                let source = ty.source();
                let validated = self.load(&ty.head.name, &ty.arguments, &source, 0)?;
                if validated == *ty {
                    return Ok(validated);
                }
                return Err(
                    format!("conflicting checked binding for construction of {source}").into(),
                );
            }
            Some(_) => {
                return Err(format!(
                    "checked construction type disagrees with native MIR type {mir}"
                )
                .into())
            }
            None => {}
        }
        let (MirType::SumType(name) | MirType::Struct(name)) = mir else {
            return Err("kernel model construction requires an exact nominal MIR type".into());
        };
        // Only an exact nongeneric source declaration is recoverable from MIR.
        // A mangled generic name only encodes ABI, even within one function.
        if !self
            .session
            .declarations
            .iter()
            .any(|decl| decl.name == *name)
        {
            return Err(format!(
                "missing exact checked expression type for native construction `{name}`"
            )
            .into());
        }
        let source = Ty::Con(TyCon::new(name));
        let ty = self.load(name, &[], &source, 0)?;
        if ty.mir() != *mir {
            return Err(format!("wrong native MIR kind for checked nominal type `{name}`").into());
        }
        Ok(ty)
    }
}

#[derive(Clone, PartialEq, Eq)]
enum ModelType {
    Bool,
    Int,
    Unit,
    Parameter { index: usize, abi: MirType },
    List(Box<ModelType>),
    Tuple(Vec<ModelType>),
    Nominal(NominalType),
}

fn parameter_symbol(index: usize) -> String {
    format!("_mesh_t{index}")
}

fn index_symbol(index: usize) -> String {
    format!("_mesh_n{index}")
}

impl ModelType {
    fn lean(&self) -> String {
        match self {
            Self::Bool => "Bool".into(),
            Self::Int => "BitVec 64".into(),
            Self::Unit => "Unit".into(),
            Self::Parameter { index, .. } => parameter_symbol(*index),
            Self::Nominal(ty) => ty.lean(),
            Self::List(element) => match element.as_ref() {
                Self::Int | Self::List(_) => format!("List ({})", element.lean()),
                _ => format!("List {}", element.lean()),
            },
            // Lean's product notation is right associative. Parenthesize
            // each Mesh tuple to preserve nesting in any nonfinal position.
            Self::Tuple(elements) => format!(
                "({})",
                elements
                    .iter()
                    .map(Self::lean)
                    .collect::<Vec<_>>()
                    .join(" × ")
            ),
        }
    }

    fn mir(&self) -> MirType {
        match self {
            Self::Bool => MirType::Bool,
            Self::Int => MirType::Int,
            Self::List(_) | Self::Tuple(_) => MirType::Ptr,
            Self::Unit => MirType::Unit,
            Self::Parameter { abi, .. } => abi.clone(),
            Self::Nominal(ty) => ty.mir(),
        }
    }

    fn require_mir(&self, mir: &MirType) -> Result<(), EncodingError> {
        if self.mir() == *mir {
            Ok(())
        } else {
            Err(format!(
                "kernel model type {} disagrees with MIR type {mir}",
                self.lean()
            )
            .into())
        }
    }

    fn matches_call_slot(&self, mir: &MirType) -> bool {
        // A direct callee's FnPtr can retain a source tuple's shape. Its
        // declared parameter or result uses the boxed Ptr representation.
        match (self, mir) {
            (Self::Tuple(checked), MirType::Tuple(fields)) => {
                checked.len() == fields.len()
                    && checked
                        .iter()
                        .zip(fields)
                        .all(|(ty, field)| ty.matches_call_slot(field))
            }
            _ => *mir == self.mir(),
        }
    }
}

#[derive(Clone, Copy)]
enum StructuralOrigin {
    Parameter(usize),
    Smaller(usize),
}

impl StructuralOrigin {
    fn smaller(self) -> Self {
        let (Self::Parameter(index) | Self::Smaller(index)) = self;
        Self::Smaller(index)
    }
}

#[derive(Clone)]
struct Term {
    lean: String,
    ty: ModelType,
    origin: Option<StructuralOrigin>,
}

impl Term {
    fn new(lean: impl Into<String>, ty: ModelType) -> Self {
        Self {
            lean: lean.into(),
            ty,
            origin: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatternStorage {
    Standalone,
    Inline,
    TupleColumns,
}

struct Encoder<'a, 'session, 'project, 'registry> {
    function: &'a MirFunction,
    name: &'a str,
    params: &'a [ModelType],
    result: &'a ModelType,
    types: &'a mut TypeScope<'session, 'project, 'registry>,
    definition_scope: DefinitionScope,
    source_file: Option<MirSourceId>,
    expression_types: BTreeMap<(u32, u32), &'a Ty>,
    locals: Vec<(&'a str, Term)>,
    next_local: usize,
    decreasing_params: Option<Vec<usize>>,
    arm_refinement: Option<(String, Ty)>,
    arm_variable: Option<TyVar>,
    remaining: usize,
}

impl<'a, 'session, 'project, 'registry> Encoder<'a, 'session, 'project, 'registry> {
    fn define(
        function: &'a MirFunction,
        name: &'a str,
        params: &'a [ModelType],
        result: &'a ModelType,
        types: &'a mut TypeScope<'session, 'project, 'registry>,
        depth: usize,
        definition_scope: DefinitionScope,
        lowering: Option<&'a LoweredSignature>,
    ) -> Result<String, EncodingError> {
        if function.is_closure_fn || !function.captures.is_empty() {
            return Err("unsupported closure function".into());
        }
        if params.len() != function.params.len() {
            return Err("kernel model signature and MIR parameter counts differ".into());
        }
        result.require_mir(&function.return_type)?;
        let mut encoder = Self {
            function,
            name,
            params,
            result,
            types,
            definition_scope,
            source_file: lowering.and_then(|entry| entry.source_file),
            expression_types: lowering
                .into_iter()
                .flat_map(|entry| entry.expression_types.iter())
                .map(|(range, ty)| ((range.start().into(), range.end().into()), ty))
                .collect(),
            locals: Vec::new(),
            next_local: 0,
            decreasing_params: None,
            arm_refinement: None,
            arm_variable: None,
            remaining: 20_000,
        };

        let mut output = format!("def {name}");
        for index in 0..encoder.types.rigid_kinds.len() {
            let (symbol, kind) = encoder.types.rigid_binder(index);
            write!(output, " ({symbol} : {kind})").unwrap();
        }
        for (index, ((local, mir), ty)) in function.params.iter().zip(params).enumerate() {
            ty.require_mir(mir)?;
            if encoder.lookup(local).is_some() {
                return Err("duplicate kernel model parameter".into());
            }
            let mut term = Term::new(format!("_mesh_p{index}"), ty.clone());
            if matches!(ty, ModelType::List(_) | ModelType::Nominal(_)) {
                term.origin = Some(StructuralOrigin::Parameter(index));
            }
            write!(output, " ({} : {})", term.lean, ty.lean()).unwrap();
            encoder.locals.push((local, term));
        }
        let body = encoder.expression(&function.body, Some(result), depth)?;
        writeln!(output, " : {} :=\n  {}", result.lean(), body.lean).unwrap();
        if let Some(indices) = encoder.decreasing_params {
            writeln!(output, "termination_by structural _mesh_p{}", indices[0]).unwrap();
        }
        output.push('\n');
        Ok(output)
    }

    fn check_call_abi(
        name: &str,
        params: &[ModelType],
        result: &ModelType,
        mir_params: &[MirType],
        mir_result: &MirType,
    ) -> Result<(), EncodingError> {
        if params.len() != mir_params.len()
            || !params
                .iter()
                .zip(mir_params)
                .all(|(ty, mir)| ty.matches_call_slot(mir))
            || !result.matches_call_slot(mir_result)
        {
            return Err(format!("MIR call ABI disagrees with checked function `{name}`").into());
        }
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<&Term> {
        self.locals
            .iter()
            .rev()
            .find(|(local, _)| *local == name)
            .map(|(_, term)| term)
    }

    fn fresh(&mut self) -> String {
        let name = format!("_mesh_v{}", self.next_local);
        self.next_local += 1;
        name
    }

    fn checked_arm_type(&mut self, ty: &Ty) -> Result<Ty, EncodingError> {
        let Some((name, value)) = &self.arm_refinement else {
            return Ok(ty.clone());
        };
        fn bind(
            ty: &Ty,
            binder: Option<&str>,
            identity: &mut Option<TyVar>,
        ) -> Result<Ty, EncodingError> {
            match ty {
                Ty::Nat(index) => {
                    let (leaf, _) = index_parts(index)?;
                    if let NatIndex::Variable(var) = leaf {
                        let binder =
                            binder.ok_or("open Nat index in a ground kernel model case arm")?;
                        if identity.is_some_and(|prior| prior != *var) {
                            return Err(
                                "multiple checked Nat witnesses for one kernel model case arm"
                                    .into(),
                            );
                        }
                        *identity = Some(*var);
                        return Ok(Ty::Nat(
                            index.map_leaf(|_| NatIndex::Parameter(binder.into())),
                        ));
                    }
                    Ok(ty.clone())
                }
                _ => {
                    let mut error = None;
                    let result = ty.map_parts(|part| match bind(part, binder, identity) {
                        Ok(ty) => ty,
                        Err(reason) => {
                            error = Some(reason);
                            part.clone()
                        }
                    });
                    error.map_or(Ok(result), Err)
                }
            }
        }
        let ty = ty.replace_cons(&mut |con| {
            (con.indexed_origin.is_none() && con.name == *name).then(|| value.clone())
        });
        bind(
            &ty,
            self.types
                .arm_binder
                .as_ref()
                .map(|(binder, _)| binder.as_str()),
            &mut self.arm_variable,
        )
    }

    fn expression(
        &mut self,
        expr: &'a MirExpr,
        expected: Option<&ModelType>,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        let mut translate = || {
            if depth >= 96 || self.remaining == 0 {
                return Err("kernel model MIR encoding budget exhausted".into());
            }
            self.remaining -= 1;
            let checked = expr
                .source_span()
                .filter(|location| Some(location.file) == self.source_file)
                .and_then(|location| {
                    self.expression_types
                        .get(&(location.span.start, location.span.end))
                        .copied()
                })
                .map(|ty| {
                    let ty = self.checked_arm_type(ty)?;
                    self.types.source(&ty)
                })
                .transpose()?;
            if let (Some(checked), Some(expected)) = (&checked, expected) {
                if checked != expected {
                    return Err(format!(
                        "conflicting checked expression type {}, expected {}",
                        checked.lean(),
                        expected.lean(),
                    )
                    .into());
                }
            }
            let expected = checked.as_ref().or(expected);
            let term = self.encode(expr, expected, depth + 1)?;
            match expr {
                // A let stores its initializer's MIR type and returns its continuation.
                MirExpr::Let { .. } => {}
                // MIR reports Never for the jump. The stored call type is its final
                // result when represented as a Lean call.
                MirExpr::TailCall { ty, .. } => term.ty.require_mir(ty)?,
                _ => term.ty.require_mir(expr.ty())?,
            }
            if let Some(expected) = expected.filter(|&ty| &term.ty != ty) {
                return Err(format!(
                    "expected kernel model type {}, got {}",
                    expected.lean(),
                    term.ty.lean()
                )
                .into());
            }
            Ok(term)
        };
        translate().map_err(|mut error: EncodingError| {
            error.location = error.location.or(expr.source_span());
            error
        })
    }

    fn encode(
        &mut self,
        expr: &'a MirExpr,
        expected: Option<&ModelType>,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        Ok(match expr {
            MirExpr::BoolLit(value, _) => Term::new(value.to_string(), ModelType::Bool),
            MirExpr::IntLit(value, _) => Term::new(format!("{}#64", *value as u64), ModelType::Int),
            MirExpr::Unit => Term::new("()", ModelType::Unit),
            MirExpr::Var(name, _) => {
                let term = self
                    .lookup(name)
                    .cloned()
                    .ok_or("unsupported non-local MIR value")?;
                if let Some((index, value)) = &self.arm_refinement {
                    if self.types.substitute(&term.ty, index, value)? != term.ty {
                        return Err("unsupported reference to a dependent local outside its kernel model case arm".into());
                    }
                }
                term
            }
            MirExpr::UnaryOp { op, operand, .. } => {
                let (ty, operator) = match op {
                    UnaryOp::Neg => (ModelType::Int, "-"),
                    UnaryOp::Not => (ModelType::Bool, "!"),
                };
                let operand = self.expression(operand, Some(&ty), depth)?;
                Term::new(format!("({operator}{})", operand.lean), ty)
            }
            MirExpr::BinOp {
                op: BinOp::Concat,
                lhs,
                rhs,
                ..
            } => self.list_concat(lhs, rhs, expected, depth)?,
            MirExpr::BinOp { op, lhs, rhs, .. } => self.binary(*op, lhs, rhs, depth)?,
            MirExpr::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                let cond = self.expression(cond, Some(&ModelType::Bool), depth)?;
                let yes = self.expression(then_body, expected, depth)?;
                let no = self.expression(else_body, Some(&yes.ty), depth)?;
                Term::new(
                    format!("(if {} then {} else {})", cond.lean, yes.lean, no.lean),
                    yes.ty,
                )
            }
            MirExpr::Let {
                name,
                ty,
                value,
                body,
            } => {
                let value = self.expression(value, None, depth)?;
                value.ty.require_mir(ty)?;
                let symbol = self.fresh();
                let mut local = Term::new(symbol.clone(), value.ty);
                local.origin = value.origin;
                self.locals.push((name, local));
                let body = self.expression(body, expected, depth);
                self.locals.pop();
                let body = body?;
                Term::new(
                    format!("(let {symbol} := {}; {})", value.lean, body.lean),
                    body.ty,
                )
            }
            MirExpr::Block(expressions, _) => {
                let Some((last, discarded)) = expressions.split_last() else {
                    return Ok(Term::new("()", ModelType::Unit));
                };
                let mut block = String::from("(");
                for step in discarded {
                    let step = self.expression(step, None, depth)?;
                    write!(block, "let _ := {}; ", step.lean).unwrap();
                }
                let last = self.expression(last, expected, depth)?;
                write!(block, "{})", last.lean).unwrap();
                Term::new(block, last.ty)
            }
            MirExpr::ListLit { elements, .. } => self.list_literal(elements, expected, depth)?,
            MirExpr::ConstructVariant {
                type_name,
                variant,
                fields,
                ty,
                ..
            } => {
                let nominal = self.types.construction_type(ty, expected)?;
                if nominal.kind != NominalKind::Sum || nominal.abi_name != *type_name {
                    return Err(
                        "kernel model variant construction disagrees with its nominal type".into(),
                    );
                }
                let (symbol, types) =
                    self.types.session.nominals[&nominal.key].constructor(variant)?;
                if types.len() != fields.len() {
                    return Err(
                        "kernel model variant construction has the wrong field count".into(),
                    );
                }
                let mut values = Vec::new();
                for (field, (_, checked)) in fields.iter().zip(&types) {
                    values.push(self.expression(field, Some(&checked.ty), depth)?.lean);
                }
                Term::new(
                    Self::constructor_application(&nominal, symbol, values),
                    ModelType::Nominal(nominal),
                )
            }
            MirExpr::StructLit {
                name, fields, ty, ..
            } => self.struct_literal(name, fields, ty, expected, depth)?,
            MirExpr::FieldAccess { object, field, .. } => {
                self.field_access(object, field, depth)?
            }
            MirExpr::Match {
                scrutinee, arms, ..
            } => self.match_expression(scrutinee, arms, expected, depth)?,
            MirExpr::Call { func, args, .. } => {
                let MirExpr::Var(name, MirType::FnPtr(params, result)) = func.as_ref() else {
                    return Err("unsupported indirect MIR call".into());
                };
                if self.lookup(name).is_some() {
                    return Err("unsupported MIR call through a local binding".into());
                }
                if name == "mesh_list_new" && params.is_empty() && **result == MirType::Ptr {
                    if self.function.name == *name || !args.is_empty() {
                        return Err("invalid kernel model empty list call".into());
                    }
                    Self::empty_list(expected)?
                } else if name == "mesh_list_concat"
                    && params.as_slice() == [MirType::Ptr, MirType::Ptr]
                    && **result == MirType::Ptr
                {
                    let [left, right] = args.as_slice() else {
                        return Err("invalid kernel model list concatenation call".into());
                    };
                    self.list_concat(left, right, expected, depth)?
                } else if name == "__mesh_make_tuple" {
                    if args.len() < 2
                        || params.len() != args.len()
                        || !(params.iter().all(|ty| *ty == MirType::Int)
                            || params.iter().eq(args.iter().map(MirExpr::ty)))
                        || **result != MirType::Ptr
                    {
                        return Err("invalid kernel model tuple allocation intrinsic".into());
                    }
                    self.tuple_literal(args, expected, depth)?
                } else if matches!(
                    name.as_str(),
                    "mesh_tuple_first" | "mesh_tuple_second" | "mesh_tuple_nth" | "mesh_tuple_size"
                ) && **result == MirType::Int
                {
                    self.tuple_access(name, params, args, depth)?
                } else if name.starts_with("mesh_") || name.starts_with("__mesh_") {
                    // Native codegen intercepts runtime and intrinsic names before direct calls.
                    return Err(format!("unsupported MIR call to `{name}`").into());
                } else if name == &self.function.name
                    || self
                        .types
                        .recursive_aliases
                        .is_some_and(|aliases| aliases.contains(name))
                {
                    Self::check_call_abi(name, self.params, self.result, params, result)?;
                    self.recur(args, depth)?
                } else {
                    if self.definition_scope != DefinitionScope::Project {
                        return Err(format!("unsupported MIR call to `{name}`").into());
                    }
                    if args.len() != params.len() {
                        return Err(format!("MIR call to `{name}` has the wrong arity").into());
                    }
                    let helper = self.types.session.helper(name, depth)?;
                    Self::check_call_abi(name, &helper.params, &helper.result, params, result)?;
                    // Native calls evaluate every argument, even when the callee ignores it.
                    let arguments = args
                        .iter()
                        .zip(&helper.params)
                        .map(|(arg, ty)| {
                            self.expression(arg, Some(ty), depth).map(|term| term.lean)
                        })
                        .collect::<Result<_, _>>()?;
                    Term::new(Self::application(helper.symbol, arguments), helper.result)
                }
            }
            MirExpr::TailCall { args, .. } if self.function.has_tail_calls => {
                self.recur(args, depth)?
            }
            MirExpr::Shaped { value, .. } => self.expression(value, expected, depth)?,
            _ => {
                return Err(format!(
                    "unsupported kernel model MIR expression of type {}",
                    expr.ty()
                )
                .into());
            }
        })
    }

    fn application(mut symbol: String, fields: Vec<String>) -> String {
        if fields.is_empty() {
            return symbol;
        }
        for field in fields {
            write!(symbol, " ({field})").unwrap();
        }
        format!("({symbol})")
    }

    fn constructor_application(ty: &NominalType, symbol: String, fields: Vec<String>) -> String {
        if ty.head.indexed_origin.is_some() {
            let value = Self::application(symbol, fields);
            return format!("({value} : {})", ty.lean());
        }
        if ty.lean_arguments.is_empty() {
            return Self::application(symbol, fields);
        }
        let mut arguments = ty.lean_arguments.clone();
        arguments.extend(fields);
        Self::application(format!("@{symbol}"), arguments)
    }

    fn struct_literal(
        &mut self,
        name: &str,
        fields: &'a [(String, MirExpr)],
        mir: &MirType,
        expected: Option<&ModelType>,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        let nominal = self.types.construction_type(mir, expected)?;
        if nominal.kind != NominalKind::Struct || nominal.abi_name != name {
            return Err("kernel model struct construction disagrees with its nominal type".into());
        }
        let (symbol, types) = self.types.session.nominals[&nominal.key].constructor("mk")?;
        if fields.len() != types.len() {
            return Err(
                "kernel model struct literal must initialize every field exactly once".into(),
            );
        }
        let mut values = vec![None; fields.len()];
        let mut bindings = String::new();
        for (name, field) in fields {
            let index = types
                .iter()
                .position(|(declared, _)| declared.as_ref() == Some(name))
                .ok_or_else(|| format!("unknown kernel model struct field `{name}`"))?;
            if values[index].is_some() {
                return Err(format!("duplicate kernel model struct field `{name}`").into());
            }
            let field = self.expression(field, Some(&types[index].1.ty), depth)?;
            let local = self.fresh();
            write!(bindings, "let {local} := {}; ", field.lean).unwrap();
            values[index] = Some(local);
        }
        let value = Self::constructor_application(
            &nominal,
            symbol,
            values.into_iter().map(Option::unwrap).collect(),
        );
        Ok(Term::new(
            format!("({bindings}{value})"),
            ModelType::Nominal(nominal),
        ))
    }

    fn field_access(
        &mut self,
        object: &'a MirExpr,
        field: &str,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        let object_ty = object.ty();
        let value = self.expression(object, None, depth)?;
        value.ty.require_mir(object_ty)?;
        let ModelType::Nominal(nominal) = value.ty else {
            return Err("kernel model field access requires an exact checked struct type".into());
        };
        if nominal.kind != NominalKind::Struct {
            return Err("kernel model field access is only defined on structs".into());
        }
        let (symbol, types) = self.types.session.nominals[&nominal.key].constructor("mk")?;
        let selected = types
            .iter()
            .position(|(name, _)| name.as_deref() == Some(field))
            .ok_or_else(|| format!("unknown kernel model struct field `{field}`"))?;
        let ty = types[selected].1.ty.clone();
        let mut fields = vec!["_".to_owned(); types.len()];
        let local = self.fresh();
        fields[selected] = local.clone();
        let pattern = Self::application(symbol, fields);
        Ok(Term::new(
            format!("(match {} with | {pattern} => {local})", value.lean),
            ty,
        ))
    }

    fn binary(
        &mut self,
        op: BinOp,
        lhs: &'a MirExpr,
        rhs: &'a MirExpr,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        let left = self.expression(lhs, None, depth)?;
        let right = self.expression(rhs, Some(&left.ty), depth)?;
        let (l, r) = (left.lean, right.lean);
        let ty = left.ty;
        use BinOp::*;
        let (text, result) = match (op, ty) {
            (Eq, ModelType::Bool | ModelType::Int) => (format!("({l} == {r})"), ModelType::Bool),
            (NotEq, ModelType::Bool | ModelType::Int) => {
                (format!("(!({l} == {r}))"), ModelType::Bool)
            }
            (And, ModelType::Bool) => (format!("({l} && {r})"), ModelType::Bool),
            (Or, ModelType::Bool) => (format!("({l} || {r})"), ModelType::Bool),
            (Add, ModelType::Int) => (format!("({l} + {r})"), ModelType::Int),
            (Sub, ModelType::Int) => (format!("({l} - {r})"), ModelType::Int),
            (Mul, ModelType::Int) => (format!("({l} * {r})"), ModelType::Int),
            (Lt, ModelType::Int) => (format!("(BitVec.slt {l} {r})"), ModelType::Bool),
            (Gt, ModelType::Int) => (format!("(BitVec.slt {r} {l})"), ModelType::Bool),
            (LtEq, ModelType::Int) => (format!("(BitVec.sle {l} {r})"), ModelType::Bool),
            (GtEq, ModelType::Int) => (format!("(BitVec.sle {r} {l})"), ModelType::Bool),
            _ => return Err(format!("unsupported kernel model binary operator {op:?}").into()),
        };
        Ok(Term::new(text, result))
    }

    fn empty_list(expected: Option<&ModelType>) -> Result<Term, EncodingError> {
        let Some(ty @ ModelType::List(_)) = expected else {
            return Err("empty list needs a concrete List<T> context".into());
        };
        Ok(Term::new(format!("([] : {})", ty.lean()), ty.clone()))
    }

    fn list_literal(
        &mut self,
        elements: &'a [MirExpr],
        expected: Option<&ModelType>,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        let Some((first, rest)) = elements.split_first() else {
            return Self::empty_list(expected);
        };
        let hint = match expected {
            Some(ModelType::List(element)) => Some(element.as_ref()),
            _ => None,
        };
        let first = self.expression(first, hint, depth)?;
        let mut elements = vec![first.lean];
        for element in rest {
            elements.push(self.expression(element, Some(&first.ty), depth)?.lean);
        }
        Ok(Term::new(
            format!("[{}]", elements.join(", ")),
            ModelType::List(Box::new(first.ty)),
        ))
    }

    fn list_concat(
        &mut self,
        lhs: &'a MirExpr,
        rhs: &'a MirExpr,
        expected: Option<&ModelType>,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        let left = self.expression(lhs, expected, depth)?;
        if !matches!(left.ty, ModelType::List(_)) {
            return Err("kernel model concatenation requires lists".into());
        }
        let right = self.expression(rhs, Some(&left.ty), depth)?;
        Ok(Term::new(
            format!("({} ++ {})", left.lean, right.lean),
            left.ty,
        ))
    }

    fn tuple_literal(
        &mut self,
        elements: &'a [MirExpr],
        expected: Option<&ModelType>,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        let hints = match expected {
            Some(ModelType::Tuple(fields)) if fields.len() == elements.len() => Some(fields),
            Some(_) => return Err("kernel model tuple construction disagrees with its type".into()),
            None => None,
        };
        let mut values = Vec::new();
        let mut types = Vec::new();
        for (index, element) in elements.iter().enumerate() {
            let field = self.expression(element, hints.map(|fields| &fields[index]), depth)?;
            values.push(field.lean);
            types.push(field.ty);
        }
        Ok(Term::new(
            format!("({})", values.join(", ")),
            ModelType::Tuple(types),
        ))
    }

    fn tuple_access(
        &mut self,
        name: &str,
        params: &[MirType],
        args: &'a [MirExpr],
        depth: usize,
    ) -> Result<Term, EncodingError> {
        let (object, index) = match (name, params, args) {
            ("mesh_tuple_first", [MirType::Ptr], [object]) => (object, Some(0)),
            ("mesh_tuple_second", [MirType::Ptr], [object]) => (object, Some(1)),
            ("mesh_tuple_size", [MirType::Ptr], [object]) => (object, None),
            (
                "mesh_tuple_nth",
                [MirType::Ptr, MirType::Int],
                [object, MirExpr::IntLit(i, MirType::Int)],
            ) => {
                let index = usize::try_from(*i).map_err(|_| "invalid kernel model tuple index")?;
                (object, Some(index))
            }
            _ => {
                return Err(
                    "kernel model tuple access requires the native ABI and a constant index".into(),
                )
            }
        };
        let tuple = self.expression(object, None, depth)?;
        let ModelType::Tuple(fields) = tuple.ty else {
            return Err("kernel model tuple access requires a concrete tuple".into());
        };
        let Some(index) = index else {
            return Ok(Term::new(
                format!("(let _ := {}; {}#64)", tuple.lean, fields.len()),
                ModelType::Int,
            ));
        };
        let selected = fields
            .get(index)
            .ok_or("kernel model tuple index out of bounds")?;
        let tail = ".2".repeat(index);
        let select = if index + 1 == fields.len() { "" } else { ".1" };
        Ok(Term::new(
            format!("({}){tail}{select}", tuple.lean),
            selected.clone(),
        ))
    }

    fn match_expression(
        &mut self,
        scrutinee: &'a MirExpr,
        arms: &'a [MirMatchArm],
        expected: Option<&ModelType>,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        if arms.is_empty() {
            return Err("empty kernel model case".into());
        }
        let subject = self.expression(scrutinee, None, depth)?;
        subject.ty.require_mir(scrutinee.ty())?;
        if let ModelType::Nominal(nominal) = &subject.ty {
            let definition = &self.types.session.nominals[&nominal.key];
            if arms.iter().any(|arm| {
                matches!(&arm.pattern, MirPattern::Constructor { variant, .. }
                    if definition.constructors.iter().any(|ctor| ctor.name == *variant && ctor.needs_refinement))
            }) {
                return self.dependent_match(scrutinee, &subject, nominal, arms, expected, depth);
            }
        }
        let storage = match scrutinee {
            // Native codegen can put a direct tuple scrutinee's inputs
            // in inline stack columns instead of allocating a boxed tuple.
            MirExpr::Call { func, .. } if matches!(func.as_ref(), MirExpr::Var(name, _) if name == "__mesh_make_tuple") => {
                PatternStorage::TupleColumns
            }
            _ => PatternStorage::Standalone,
        };
        let mut output = format!("(match {} with", subject.lean);
        let mut result = expected.cloned();
        for arm in arms {
            if let Some(guard) = &arm.guard {
                return Err(EncodingError {
                    reason: "unsupported guard on a kernel model pattern".into(),
                    location: guard.source_span(),
                });
            }
            let count = self.locals.len();
            let pattern = self.pattern(
                &arm.pattern,
                &subject.ty,
                subject.origin,
                storage,
                count,
                depth,
            )?;
            let body = self.expression(&arm.body, result.as_ref(), depth);
            self.locals.truncate(count);
            let body = body?;
            write!(output, " | {pattern} => {}", body.lean).unwrap();
            result = Some(body.ty);
        }
        output.push(')');
        Ok(Term::new(
            output,
            result.expect("each arm determines a type"),
        ))
    }

    fn dependent_match(
        &mut self,
        scrutinee: &'a MirExpr,
        subject: &Term,
        nominal: &NominalType,
        arms: &'a [MirMatchArm],
        expected: Option<&ModelType>,
        depth: usize,
    ) -> Result<Term, EncodingError> {
        if self.arm_refinement.is_some() {
            return Err(
                "nested dependent Nat index refinements are unsupported in the kernel model".into(),
            );
        }
        let result =
            expected.ok_or("a dependent kernel model case needs an exact checked result type")?;
        let (family, _) = self.types.checked_nominal(&nominal.head.name)?;
        let [position] = family.index_positions.as_slice() else {
            return Err("dependent kernel model cases require exactly one Nat index".into());
        };
        let Ty::Nat(NatIndex::Parameter(name)) = &nominal.arguments[*position] else {
            return Err("dependent kernel model cases require a bare rigid Nat index".into());
        };
        let index = self.types.render_index(&nominal.arguments[*position])?;
        let motive = format!("({index} : Nat) → {} → {}", nominal.lean(), result.lean());
        let mut output = format!("(match (motive := {motive}) {index}, {} with", subject.lean);
        for arm in arms {
            if let Some(guard) = &arm.guard {
                return Err(EncodingError {
                    reason: "unsupported guard on a dependent kernel model pattern".into(),
                    location: guard.source_span(),
                });
            }
            let MirPattern::Constructor { variant, .. } = &arm.pattern else {
                return Err(
                    "dependent kernel model cases require explicit constructor arms".into(),
                );
            };
            let constructor = family
                .constructors
                .iter()
                .find(|ctor| ctor.name == *variant)
                .and_then(|ctor| ctor.indexed.as_ref())
                .ok_or("missing checked indexed constructor in a dependent kernel model case")?;
            let Ty::App(_, constructor_args) = &constructor.result else {
                return Err(
                    "invalid checked constructor result in a dependent kernel model case".into(),
                );
            };
            let Ty::Nat(constructor_index) = &constructor_args[*position] else {
                return Err(
                    "missing checked constructor Nat index in a dependent kernel model case".into(),
                );
            };
            let (index_pattern, replacement) = match IndexForm::read(constructor_index)? {
                ground @ IndexForm::Ground(value) => {
                    (value.to_string(), Ty::Nat(ground.into_index()))
                }
                IndexForm::Rigid(parameter, successors)
                    if constructor.parameters.as_slice() == [parameter] =>
                {
                    let symbol = self.fresh();
                    let binder = format!("\0mesh_model_arm{symbol}");
                    if self.types.parameters.contains(&binder) {
                        return Err(
                            "rigid kernel model parameter collides with a case arm binder".into(),
                        );
                    }
                    let mut index_pattern = symbol.clone();
                    for _ in 0..successors {
                        index_pattern = format!("(Nat.succ {index_pattern})");
                    }
                    let replacement = Ty::Nat(IndexForm::Rigid(&binder, successors).into_index());
                    self.types.arm_binder = Some((binder, symbol));
                    (index_pattern, replacement)
                }
                _ => {
                    return Err(
                        "unsupported constructor Nat witnesses in a dependent kernel model case"
                            .into(),
                    )
                }
            };
            self.arm_refinement = Some((name.clone(), replacement.clone()));
            let scope = self.locals.len();
            let branch = (|| {
                let ty = self.types.substitute(&subject.ty, name, &replacement)?;
                let ModelType::Nominal(refined) = &ty else {
                    unreachable!("an index substitution preserves the nominal head");
                };
                if nominal
                    .arguments
                    .iter()
                    .zip(&refined.arguments)
                    .enumerate()
                    .any(|(arg, (before, after))| arg != *position && before != after)
                {
                    return Err("dependent kernel model case refines a family type argument".into());
                }
                let mut pattern = self.pattern(
                    &arm.pattern,
                    &ty,
                    subject.origin,
                    PatternStorage::Standalone,
                    scope,
                    depth,
                )?;
                if let Some(local) = scrutinee.var_name() {
                    let symbol = self.fresh();
                    pattern = format!("{symbol}@({pattern})");
                    self.locals.insert(
                        scope,
                        (
                            local,
                            Term {
                                lean: symbol,
                                ty,
                                origin: subject.origin,
                            },
                        ),
                    );
                }
                let result = self.types.substitute(result, name, &replacement)?;
                let body = self.expression(&arm.body, Some(&result), depth)?;
                Ok::<_, EncodingError>((pattern, body))
            })();
            self.locals.truncate(scope);
            self.arm_refinement = None;
            self.arm_variable = None;
            self.types.arm_binder = None;
            let (pattern, body) = branch?;
            write!(output, " | {index_pattern}, {pattern} => {}", body.lean).unwrap();
        }
        output.push(')');
        Ok(Term::new(output, result.clone()))
    }

    fn pattern(
        &mut self,
        pattern: &'a MirPattern,
        ty: &ModelType,
        origin: Option<StructuralOrigin>,
        storage: PatternStorage,
        scope: usize,
        depth: usize,
    ) -> Result<String, EncodingError> {
        if depth >= 96 || self.remaining == 0 {
            return Err("kernel model MIR encoding budget exhausted".into());
        }
        self.remaining -= 1;
        Ok(match (pattern, ty) {
            (MirPattern::Wildcard, _) => "_".into(),
            (MirPattern::Var(local, mir), _) => {
                ty.require_mir(mir)?;
                if self.locals[scope..].iter().any(|(name, _)| *name == local) {
                    return Err("duplicate kernel model pattern binding".into());
                }
                let symbol = self.fresh();
                self.locals.push((
                    local,
                    Term {
                        lean: symbol.clone(),
                        ty: ty.clone(),
                        origin,
                    },
                ));
                symbol
            }
            (MirPattern::Literal(MirLiteral::Bool(value)), ModelType::Bool) => value.to_string(),
            (MirPattern::Literal(MirLiteral::Int(value)), ModelType::Int) => {
                format!("{}#64", *value as u64)
            }
            (MirPattern::ListNil, ModelType::List(_)) => "[]".into(),
            (
                MirPattern::ListCons {
                    head,
                    tail,
                    elem_ty,
                },
                ModelType::List(element),
            ) => {
                element.require_mir(elem_ty)?;
                // The native decision tree tests length before loading either field.
                let head = self.pattern(
                    head,
                    element,
                    None,
                    PatternStorage::Standalone,
                    scope,
                    depth + 1,
                )?;
                let tail_origin = origin.map(StructuralOrigin::smaller);
                let tail = self.pattern(
                    tail,
                    ty,
                    tail_origin,
                    PatternStorage::Standalone,
                    scope,
                    depth + 1,
                )?;
                format!("({head} :: {tail})")
            }
            (MirPattern::Tuple(fields), ModelType::Tuple(types)) if fields.len() == types.len() => {
                let element_storage = if storage == PatternStorage::TupleColumns {
                    PatternStorage::Inline
                } else {
                    PatternStorage::Standalone
                };
                let fields = fields
                    .iter()
                    .zip(types)
                    .map(|(field, ty)| {
                        self.pattern(field, ty, None, element_storage, scope, depth + 1)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                format!("({})", fields.join(", "))
            }
            (MirPattern::Tuple(fields), ModelType::Unit) if fields.is_empty() => "()".into(),
            (
                MirPattern::Constructor {
                    type_name,
                    variant,
                    fields,
                    ..
                },
                ModelType::Nominal(nominal),
            ) if nominal.kind == NominalKind::Sum && nominal.abi_name == *type_name => {
                let definition = &self.types.session.nominals[&nominal.key];
                let (symbol, types) = definition.constructor(variant)?;
                if types.len() != fields.len() {
                    return Err("kernel model constructor pattern has the wrong field count".into());
                }
                let byte_overlay =
                    storage == PatternStorage::Inline && !definition.has_pointer_layout();
                let mut patterns = Vec::new();
                for (pattern, (_, field)) in fields.iter().zip(&types) {
                    let field_ty = &field.ty;
                    // An embedded byte-backed sum has ABI alignment one even
                    // when a variant overlay requires an aligned payload load.
                    if byte_overlay && !matches!(pattern, MirPattern::Wildcard) {
                        let needs_alignment = match (&field.storage, field_ty) {
                            (MirType::Bool | MirType::Unit, _) => false,
                            (MirType::SumType(_), ModelType::Nominal(inner)) => {
                                self.types.session.nominals[&inner.key].has_pointer_layout()
                            }
                            _ => true,
                        };
                        if needs_alignment {
                            return Err(format!("native inline payload of `{type_name}.{variant}` has no guaranteed alignment").into());
                        }
                    }
                    let field_storage = if matches!(field.storage, MirType::SumType(_)) {
                        PatternStorage::Inline
                    } else {
                        PatternStorage::Standalone
                    };
                    let same_family = matches!(field_ty, ModelType::Nominal(inner)
                        if nominal.head.indexed_origin.is_some() && nominal.head == inner.head
                        && nominal.arguments.iter().filter(|arg| !matches!(arg, Ty::Nat(_)))
                        .eq(inner.arguments.iter().filter(|arg| !matches!(arg, Ty::Nat(_)))));
                    let field_origin = if ty == field_ty || same_family {
                        origin.map(StructuralOrigin::smaller)
                    } else {
                        None
                    };
                    patterns.push(self.pattern(
                        pattern,
                        field_ty,
                        field_origin,
                        field_storage,
                        scope,
                        depth + 1,
                    )?);
                }
                Self::application(symbol, patterns)
            }
            (MirPattern::Struct { name, fields }, ModelType::Nominal(nominal))
                if nominal.kind == NominalKind::Struct && nominal.abi_name == *name =>
            {
                let (symbol, types) =
                    self.types.session.nominals[&nominal.key].constructor("mk")?;
                if types.len() != fields.len() {
                    return Err("kernel model struct pattern has the wrong field count".into());
                }
                let mut patterns = Vec::new();
                for ((field, mir, pattern), (declared, checked)) in fields.iter().zip(&types) {
                    if Some(field) != declared.as_ref() {
                        return Err(
                            "kernel model struct pattern disagrees with field layout".into()
                        );
                    }
                    checked.ty.require_mir(mir)?;
                    patterns.push(self.pattern(
                        pattern,
                        &checked.ty,
                        None,
                        PatternStorage::Inline,
                        scope,
                        depth + 1,
                    )?);
                }
                Self::application(symbol, patterns)
            }
            _ => return Err(format!("unsupported kernel model pattern for {}", ty.lean()).into()),
        })
    }

    fn recur(&mut self, args: &'a [MirExpr], depth: usize) -> Result<Term, EncodingError> {
        if args.len() != self.params.len() {
            return Err("kernel model recursive call has the wrong arity".into());
        }
        let substitution = if let Some((name, refined)) = &self.arm_refinement {
            let mut recursive_index = None;
            for (position, (arg, param)) in args.iter().zip(self.params).enumerate() {
                let Some(local) = arg.var_name().and_then(|local| self.lookup(local)) else {
                    continue;
                };
                if !matches!(local.origin, Some(StructuralOrigin::Smaller(root)) if root == position)
                {
                    continue;
                }
                let (ModelType::Nominal(original), ModelType::Nominal(smaller)) =
                    (param, &local.ty)
                else {
                    continue;
                };
                for (before, after) in original.arguments.iter().zip(&smaller.arguments) {
                    if matches!(before, Ty::Nat(NatIndex::Parameter(parameter)) if parameter == name)
                    {
                        if recursive_index.is_some_and(|other| other != after) {
                            return Err(
                                "recursive kernel model call has conflicting Nat indices".into()
                            );
                        }
                        recursive_index = Some(after);
                    }
                }
            }
            Some((name.clone(), recursive_index.unwrap_or(refined).clone()))
        } else {
            None
        };
        let mut arguments = Vec::new();
        for (position, parameter) in self.types.parameters.iter().enumerate() {
            let argument = match &substitution {
                Some((name, index)) if parameter == name => self.types.render_index(index)?,
                _ => self.types.rigid_binder(position).0,
            };
            arguments.push(argument);
        }
        let mut decreasing = Vec::new();
        for (index, (argument, ty)) in args.iter().zip(self.params).enumerate() {
            let ty = match &substitution {
                Some((name, value)) => self.types.substitute(ty, name, value)?,
                None => ty.clone(),
            };
            let argument = self.expression(argument, Some(&ty), depth)?;
            if matches!(argument.origin, Some(StructuralOrigin::Smaller(root)) if root == index) {
                decreasing.push(index);
            }
            arguments.push(argument.lean);
        }
        // Require the same structural parameter to decrease at every recursive
        // call. Lean independently checks the chosen parameter on elaboration.
        if let Some(previous) = &self.decreasing_params {
            decreasing.retain(|index| previous.contains(index));
        }
        if decreasing.is_empty() {
            return Err(
                "kernel model recursion must decrease a common parameter by taking its list tail or a recursive constructor field"
                    .into(),
            );
        }
        self.decreasing_params = Some(decreasing);
        let result = match &substitution {
            Some((name, value)) => self.types.substitute(self.result, name, value)?,
            None => self.result.clone(),
        };
        Ok(Term::new(
            format!("({} {})", self.name, arguments.join(" ")),
            result,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_parser::ast::item::FnDef;
    use mesh_parser::AstNode;

    fn checked_program(source: &str) -> (MirModule, TypeRegistry, BTreeMap<String, Ty>) {
        let parsed = mesh_parser::parse(source);
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        let checked = mesh_typeck::check(&parsed);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let signatures = parsed
            .tree()
            .syntax()
            .children()
            .filter_map(FnDef::cast)
            .map(|declaration| {
                let symbol = declaration.name().unwrap().text().unwrap();
                (
                    symbol,
                    checked.types[&declaration.syntax().text_range()].clone(),
                )
            })
            .collect();
        let module = mesh_codegen::mir::lower::lower_to_mir(
            &parsed,
            &checked,
            "",
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        (module, checked.type_registry, signatures)
    }

    fn lower(source: &str) -> (MirFunction, Ty) {
        let (module, _, mut signatures) = checked_program(source);
        let (name, signature) = signatures.pop_first().unwrap();
        (function(&module, &name).clone(), signature)
    }

    fn function<'a>(module: &'a MirModule, name: &str) -> &'a MirFunction {
        module.functions.iter().find(|f| f.name == name).unwrap()
    }

    #[test]
    fn checked_helper_closure_across_modules_computes_in_lean() {
        use mesh_pkg::project::{check_project, load_modules, read_file};

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(
            root.join("helpers.mpl"),
            r#"
struct Parcel do
  rows :: List<List<Int>>
end

pub fn keep<T>(items :: List<T>) -> List<T> = items

fn copy(items :: List<Int>) -> List<Int> do
  case items do
    [] -> []
    head :: rest -> [head] ++ copy(rest)
  end
end

fn pack(rows :: List<List<Int>>) -> Parcel do
  Parcel { rows: [copy([1])] ++ keep(rows) }
end

pub fn payload(rows :: List<List<Int>>) -> (List<List<Int>>, List<Bool>) do
  let first = pack(rows)
  let second = pack([])
  (first.rows ++ second.rows, keep([true]))
end
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("entry.mpl"),
            r#"
import Helpers

fn payload(rows :: List<List<Int>>) -> List<List<Int>> = [] ++ rows

pub fn run(rows :: List<List<Int>>) -> (List<List<Int>>, List<Bool>) do
  Helpers.payload(payload(Helpers.keep(rows)))
end
"#,
        )
        .unwrap();
        let lower_project = || {
            let project = load_modules(root, None, &[], false, &read_file).unwrap();
            let mut checked = check_project(&project, false);
            for result in &checked.typeck {
                assert!(result.errors.is_empty(), "{:?}", result.errors);
            }
            let mut verification = crate::VerificationLowering::default();
            let modules = crate::lower_checked_modules(
                &project,
                &mut checked,
                |_| None,
                Some(&mut verification),
            )
            .unwrap();
            (project, checked, modules, verification.signatures)
        };

        let (project, checked, modules, signatures) = lower_project();
        let declarations = super::super::nominal_declarations(&project, &checked);
        let callees = super::super::checked_callees(&project, &checked, &modules, &signatures);
        let run = callees["run"];
        let mut session = ModelSession::new(&modules, &declarations).with_callees(callees);
        let error = session
            .translate(
                run.function,
                run.registry,
                run.function,
                run.registry,
                run.signature,
            )
            .err()
            .unwrap();
        assert!(error
            .reason
            .contains("expected expression: unsupported MIR call to `payload`"));

        let (expected, _) = lower(
            "fn expected(rows :: List<List<Int>>) -> (List<List<Int>>, List<Bool>) = \
                ([[1]] ++ rows ++ [[1]], [true])",
        );
        let model = session
            .translate(
                run.function,
                run.registry,
                &expected,
                &TypeRegistry::default(),
                run.signature,
            )
            .unwrap();
        // A private and a public `payload`, `pack`, structural `copy`, and two
        // `keep` instances with the same pointer ABI are each emitted once.
        assert_eq!(
            session
                .helper_definitions()
                .matches("def mesh_helper_")
                .count(),
            6
        );
        assert!(session
            .helper_definitions()
            .contains("termination_by structural _mesh_p0"));
        assert!(model.definitions.starts_with(session.prelude()));
        assert!(
            model.definitions[session.prelude().len()..].starts_with(session.helper_definitions())
        );

        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let source = format!(
                "{}{}\ntheorem helper_computation : mesh_actual [[2#64]] = ([[1#64], [2#64], [1#64]], [true]) := rfl\n\
                 theorem legacy_model : mesh_actual [[2#64]] = mesh_expected [[2#64]] := rfl\n",
                super::super::PRELUDE,
                model.definitions,
            );
            super::super::check_artifact(
                source,
                &["helper_computation".into(), "legacy_model".into()],
                super::super::Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap();
        }

        std::fs::write(
            root.join("collision.mpl"),
            "pub fn Entry__payload(rows :: List<List<Bool>>) -> List<List<Bool>> = rows\n",
        )
        .unwrap();
        let (project, checked, modules, signatures) = lower_project();
        let declarations = super::super::nominal_declarations(&project, &checked);
        let callees = super::super::checked_callees(&project, &checked, &modules, &signatures);
        let run = callees["run"];
        let error = ModelSession::new(&modules, &declarations)
            .with_callees(callees)
            .definition(run.function, run.signature, run.registry, "mesh_actual")
            .unwrap_err();
        assert!(
            error
                .reason
                .contains("`Entry__payload`: no unique checked body"),
            "{error:?}"
        );
    }

    fn checked_helper_error(source: &str) -> EncodingError {
        use mesh_codegen::mir::lower::lower_module_to_mir_with_signatures;
        use mesh_codegen::mir::MirSourceId;

        let parse = mesh_parser::parse(source);
        assert!(parse.errors().is_empty(), "{:?}", parse.errors());
        let checked = mesh_typeck::check(&parse);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let (module, signatures) = lower_module_to_mir_with_signatures(
            &parse,
            &checked,
            "",
            &Default::default(),
            &Default::default(),
            &[],
            |_| Some(MirSourceId(0)),
        )
        .unwrap();
        let callees: BTreeMap<_, _> = signatures
            .iter()
            .map(|entry| {
                (
                    entry.symbol.as_str(),
                    CheckedCallee {
                        function: function(&module, &entry.symbol),
                        signature: &entry.signature,
                        registry: &checked.type_registry,
                        lowering: Some(entry),
                    },
                )
            })
            .collect();
        let root = callees["target"];
        ModelSession::new(std::slice::from_ref(&module), &[])
            .with_callees(callees)
            .definition(root.function, root.signature, root.registry, "mesh_actual")
            .unwrap_err()
    }

    #[test]
    fn helper_admission_checks_discarded_branches_and_native_arguments() {
        let source = r#"
fn dirty() -> Int do
  println("effect")
  2
end

fn ignore(value :: Int) -> Int = 7

fn helper(flag :: Bool) -> Int do
  if flag do
    ignore(dirty())
  else
    1
  end
  4
end

fn target() -> Int = helper(false)
"#;
        let error = checked_helper_error(source);
        assert!(
            error.reason.contains(
                "helper `helper`: helper `dirty`: unsupported MIR call to `mesh_println`"
            ),
            "{error:?}"
        );
        let location = error.location.unwrap();
        assert_eq!(location.file.0, 0);
        assert_eq!(
            location.span.start as usize,
            source.find("println").unwrap()
        );
    }

    #[test]
    fn cyclic_helper_admission_reports_the_back_edge() {
        let source = r#"
fn third(value :: Int) -> Int = target(value)
fn second(value :: Int) -> Int = third(value)
fn target(value :: Int) -> Int = second(value)
"#;
        let error = checked_helper_error(source);
        assert!(
            error.reason.contains("target -> second -> third -> target"),
            "{error:?}"
        );
        assert_eq!(
            error.location.unwrap().span.start as usize,
            source.find("target(value)").unwrap()
        );
    }

    #[test]
    fn native_nominal_payloads_and_recursive_fields_compute_in_lean() {
        let (module, registry, signatures) = checked_program(
            r#"
type Chain do
  Done
  Step(Int, Chain)
end

struct Packet do
  amount :: Int
  ready :: Bool
end

type Arrival do
  Missing
  Present(Packet)
end

fn total(chain :: Chain) -> Int do
  case chain do
    Step(amount, rest) -> amount + total(rest)
    Done -> 0
  end
end

fn last(chain :: Chain) -> Int do
  case chain do
    Step(amount, Done) -> amount
    Step(_, rest) -> last(rest)
    Done -> 0
  end
end

fn extend(amount :: Int, rest :: Chain) -> Chain do
  Step(amount, rest)
end

fn arrival(amount :: Int, ready :: Bool) -> Arrival do
  Present(Packet { ready: ready, amount: amount })
end

fn unpack(value :: Arrival) -> Int do
  case value do
    Present(Packet { amount: 0, ready: true }) -> 2
    Present(Packet { amount, ready: true }) -> amount
    Present(Packet { ready: false }) -> 0
    Missing -> 1
  end
end

fn read(packet :: Packet) -> Int do
  packet.amount
end
"#,
        );
        let names = ["total", "last", "extend", "arrival", "unpack", "read"];
        let native =
            mesh_codegen::merge_mir_modules(vec![module.clone()], 0, &names.map(str::to_owned));
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        std::fs::create_dir_all(&output).unwrap();
        let ir = tempfile::NamedTempFile::new_in(output).unwrap();
        mesh_codegen::compile_mir_to_llvm_ir(&native, &[], &[], None, &[], ir.path(), 0, None)
            .unwrap();

        let declarations = ["Packet", "Chain", "Arrival"].map(|name| NominalDeclaration {
            name: name.into(),
            registry: &registry,
        });
        let mut session = ModelSession::new(std::slice::from_ref(&module), &declarations);
        let mut definitions = String::new();
        for name in names {
            let definition = session
                .definition(function(&module, name), &signatures[name], &registry, name)
                .unwrap();
            definitions.push_str(&definition);
        }
        let done = session
            .constructor(&Ty::Con(TyCon::new("Chain")), &registry, "Done")
            .unwrap();
        let packet_type = Ty::Con(TyCon::new("Packet"));
        let packet = session.constructor(&packet_type, &registry, "mk").unwrap();
        let packet_type = session.source_type(&packet_type, &registry).unwrap();
        assert_eq!(session.prelude().matches("inductive ").count(), 3);

        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let artifact = format!(
                "{}{}{definitions}
theorem model_total : total (extend 7#64 (extend 5#64 {done})) = 12#64 := rfl
theorem model_last : last (extend 7#64 (extend 5#64 {done})) = 5#64 := rfl
theorem model_unpacked : unpack (arrival 9#64 true) = 9#64 ∧
    unpack (arrival 9#64 false) = 0#64 ∧
    unpack (arrival 0#64 true) = 2#64 := ⟨rfl, rfl, rfl⟩
theorem model_read (amount : BitVec 64) (ready : Bool) :
    read ({packet} amount ready : {packet_type}) = amount := rfl",
                super::super::PRELUDE,
                session.prelude(),
            );
            super::super::check_artifact(
                artifact,
                &[
                    "model_total".into(),
                    "model_last".into(),
                    "model_unpacked".into(),
                    "model_read".into(),
                ],
                super::super::Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn generic_nominals_preserve_checked_types_with_shared_native_layouts() {
        use mesh_codegen::mir::lower::lower_module_to_mir_with_signatures;

        let parsed = mesh_parser::parse(
            r#"
type Tree<T> do
  Leaf(T)
  Fork(Tree<T>, Tree<T>)
end

struct TreeBox<T> do
  item :: T
end

fn firstInts(tree :: Tree<List<Int>>) -> List<Int> do
  case tree do
    Leaf(items) -> items
    Fork(left, _) -> firstInts(left)
  end
end

fn firstBools(tree :: Tree<List<Bool>>) -> List<Bool> do
  case tree do
    Leaf(items) -> items
    Fork(left, _) -> firstBools(left)
  end
end

fn isSeven(tree :: Tree<Int>) -> Bool do
  case tree do
    Leaf(7) -> true
    _ -> false
  end
end

fn sample(i :: Int, ready :: Bool) -> (List<Int>, List<Bool>, Bool) do
  let ints = Leaf([i])
  let bools = Leaf([ready])
  let intBox = TreeBox { item: Fork(ints, ints) }
  let boolBox = TreeBox { item: Fork(bools, bools) }
  let seven = Leaf(7)
  case boolBox do
    TreeBox { item } -> (firstInts(intBox.item), firstBools(item), isSeven(seven))
  end
end
"#,
        );
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        let checked = mesh_typeck::check(&parsed);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let (module, signatures) = lower_module_to_mir_with_signatures(
            &parsed,
            &checked,
            "",
            &Default::default(),
            &Default::default(),
            &[],
            |_| Some(MirSourceId(0)),
        )
        .unwrap();
        assert_eq!(
            function(&module, "firstInts").params[0].1,
            MirType::SumType("Tree_Ptr".into())
        );
        assert_eq!(
            function(&module, "firstInts").params,
            function(&module, "firstBools").params
        );
        assert!(module.sum_types.iter().any(|def| def.name == "Tree"));
        assert!(!module.sum_types.iter().any(|def| def.name == "Tree_Ptr"));
        assert!(module
            .structs
            .iter()
            .any(|def| def.name == "TreeBox_Tree_Ptr"));

        let native = mesh_codegen::merge_mir_modules(vec![module.clone()], 0, &["sample".into()]);
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        let ir = tempfile::NamedTempFile::new_in(output).unwrap();
        mesh_codegen::compile_mir_to_llvm_ir(&native, &[], &[], None, &[], ir.path(), 0, None)
            .unwrap();
        let llvm = std::fs::read_to_string(ir.path()).unwrap();
        assert!(llvm.contains("@firstInts(") && llvm.contains("@firstBools("));

        let registry = &checked.type_registry;
        let declarations = ["Tree", "TreeBox"].map(|name| NominalDeclaration {
            name: name.into(),
            registry,
        });
        let callees: BTreeMap<_, _> = signatures
            .iter()
            .map(|entry| {
                (
                    entry.symbol.as_str(),
                    CheckedCallee {
                        function: function(&module, &entry.symbol),
                        signature: &entry.signature,
                        registry,
                        lowering: Some(entry),
                    },
                )
            })
            .collect();
        let root = callees["sample"];
        let mut session =
            ModelSession::new(std::slice::from_ref(&module), &declarations).with_callees(callees);
        let model = session
            .definition(root.function, root.signature, registry, "sample")
            .unwrap();
        let int_tree = callee_param(&signatures, "firstInts");
        let bool_tree = callee_param(&signatures, "firstBools");
        let int_symbol = session.source_type(int_tree, registry).unwrap();
        let bool_symbol = session.source_type(bool_tree, registry).unwrap();
        assert_ne!(int_symbol, bool_symbol);
        assert_eq!(
            session.constructor(int_tree, registry, "Leaf").unwrap(),
            session.constructor(bool_tree, registry, "Leaf").unwrap(),
        );
        assert_eq!(session.prelude().matches("inductive ").count(), 2);

        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let source = format!(
                "{}{}{}{model}\ntheorem generic_computation : sample 5#64 true = ([5#64], [true], true) := rfl\n",
                super::super::PRELUDE,
                session.prelude(),
                session.helper_definitions(),
            );
            super::super::check_artifact(
                source,
                &["generic_computation".into()],
                super::super::Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap();
        }

        let error = ModelSession::new(std::slice::from_ref(&module), &declarations)
            .definition(root.function, root.signature, registry, "sample")
            .unwrap_err();
        assert!(
            error
                .reason
                .contains("missing exact checked expression type"),
            "{error:?}"
        );

        let mut conflicting = root.lowering.unwrap().clone();
        let leaf = parsed
            .syntax()
            .descendants()
            .find(|node| {
                node.kind() == mesh_parser::SyntaxKind::CALL_EXPR && node.text() == "Leaf([i])"
            })
            .unwrap();
        *conflicting
            .expression_types
            .get_mut(&leaf.text_range())
            .unwrap() = bool_tree.clone();
        let conflicting_root = CheckedCallee {
            lowering: Some(&conflicting),
            ..root
        };
        let error = ModelSession::new(std::slice::from_ref(&module), &declarations)
            .with_callees(BTreeMap::from([("sample", conflicting_root)]))
            .definition(root.function, root.signature, registry, "sample")
            .unwrap_err();
        assert!(
            error.reason.contains("conflicting checked expression type"),
            "{error:?}"
        );

        let mut shadowed = module.clone();
        let mut exact = module
            .sum_types
            .iter()
            .find(|def| def.name == "Tree")
            .unwrap()
            .clone();
        exact.name = "Tree_Ptr".into();
        shadowed.sum_types.push(exact);
        let error = ModelSession::new(std::slice::from_ref(&shadowed), &declarations)
            .source_type(int_tree, registry)
            .unwrap_err();
        assert!(
            error.reason.contains("does not uniquely fall back"),
            "{error:?}"
        );
    }

    #[test]
    fn native_indexed_families_keep_ground_fields_and_shared_layouts() {
        let parsed = mesh_parser::parse(
            r#"
pub type Indexed_Vec<n :: Nat, T> do
  Empty -> Indexed_Vec<0, T>
  Push<k :: Nat>(T, Indexed_Vec<k, T>) -> Indexed_Vec<Succ<k>, T>
end

fn first(values :: Indexed_Vec<1, Int>) -> Int do
  case values do
    Push(head, _) -> head
  end
end

fn rest(values :: Indexed_Vec<1, Int>) -> Indexed_Vec<0, Int> do
  case values do
    Push(_, tail) -> tail
  end
end

fn singleton(value :: Int) -> Indexed_Vec<1, Int> do
  Push(value, Empty)
end
"#,
        );
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        let checked = mesh_typeck::check(&parsed);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let (module, signatures) = mesh_codegen::mir::lower::lower_module_to_mir_with_signatures(
            &parsed,
            &checked,
            "",
            &Default::default(),
            &Default::default(),
            &[],
            |_| Some(MirSourceId(0)),
        )
        .unwrap();
        let registry = &checked.type_registry;
        let declarations = [NominalDeclaration {
            name: "Indexed_Vec".into(),
            registry,
        }];
        let callees: BTreeMap<_, _> = signatures
            .iter()
            .map(|entry| {
                let callee = CheckedCallee {
                    function: function(&module, &entry.symbol),
                    signature: &entry.signature,
                    registry,
                    lowering: Some(entry),
                };
                (entry.symbol.as_str(), callee)
            })
            .collect();
        let first = callees["first"];
        let rest = callees["rest"];
        let Ty::Fun(params, empty) = rest.signature else {
            panic!("expected rest's signature")
        };
        let one = &params[0];
        assert_eq!(resolve_type(one, registry), resolve_type(empty, registry));
        let mut session =
            ModelSession::new(std::slice::from_ref(&module), &declarations).with_callees(callees);
        let first = session.import(&first.function.name).unwrap();
        let rest = session.import(&rest.function.name).unwrap();
        let singleton = session.import("singleton").unwrap();
        let push = session.constructor(one, registry, "Push").unwrap();
        let empty_ctor = session.constructor(empty, registry, "Empty").unwrap();
        let rendered_empty = session.source_type(empty, registry).unwrap();
        let (source_family, _) = session.nominal_family("Indexed_Vec", registry).unwrap();
        assert!(session.prelude().contains(" : Nat → Type where"));
        assert_eq!(session.prelude().matches("inductive ").count(), 1);
        assert_eq!(
            session.nominal_constructors(empty, registry).unwrap(),
            [("Empty".into(), empty_ctor.clone(), vec![])]
        );
        assert_eq!(
            session.nominal_constructors(one, registry).unwrap(),
            [(
                "Push".into(),
                push.clone(),
                vec!["BitVec 64".into(), rendered_empty.clone()]
            )]
        );
        assert!(session
            .constructor(one, registry, "Empty")
            .unwrap_err()
            .reason
            .contains("cannot inhabit"));
        assert!(session
            .constructor(empty, registry, "Push")
            .unwrap_err()
            .reason
            .contains("cannot inhabit"));

        let mut open = one.clone();
        let Ty::App(_, arguments) = &mut open else {
            panic!("expected Vec application")
        };
        arguments[0] = Ty::Nat(NatIndex::Parameter("n".into()));
        assert!(session
            .source_type(&open, registry)
            .unwrap_err()
            .reason
            .contains("open Nat index"));

        let mut bad_layout = module.clone();
        let native_vec = bad_layout
            .sum_types
            .iter_mut()
            .find(|def| def.name == "Indexed_Vec")
            .unwrap();
        native_vec.variants[1].fields[0] = MirType::Int;
        let error = ModelSession::new(&[bad_layout], &declarations)
            .source_type(empty, registry)
            .unwrap_err();
        assert!(
            error.reason.contains("native MIR layout disagree"),
            "{error:?}"
        );

        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let source = format!(
                "{}{}{}
theorem indexed_first (head : BitVec 64) (tail : {source_family} 0 (BitVec 64)) :
    {first} ({push} head tail) = head := rfl
theorem indexed_rest (head : BitVec 64) :
    {rest} ({singleton} head) = ({empty_ctor} : {rendered_empty}) := rfl
",
                super::super::PRELUDE,
                session.prelude(),
                session.helper_definitions(),
            );
            super::super::check_artifact(
                source,
                &["indexed_first".into(), "indexed_rest".into()],
                super::super::Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn rigid_indexed_functions_preserve_constructor_field_types() {
        let parsed = mesh_parser::parse(
            r#"
type IndexedVec<T, n :: Nat> do
  Empty -> IndexedVec<T, 0>
  Push<k :: Nat>(T, IndexedVec<T, k>) -> IndexedVec<T, Succ<k>>
end

fn first<T, n :: Nat>(values :: IndexedVec<T, Succ<n>>) -> T do
  case values do
    Push(head, _) -> head
  end
end

fn rest<T, n :: Nat>(values :: IndexedVec<T, Succ<n>>) -> IndexedVec<T, n> do
  case values do
    Push(_, tail) -> tail
  end
end

fn rebuild<T, n :: Nat>(values :: IndexedVec<T, n>) -> IndexedVec<T, n> do
  case values do
    Empty -> Empty
    Push(head, tail) -> Push(head, tail)
  end
end

fn copy_int<n :: Nat>(values :: IndexedVec<Int, n>) -> IndexedVec<Int, n> do
  case values do
    Empty -> Empty
    Push(head, tail) -> Push(head, copy_int(tail))
  end
end

fn copy<T, n :: Nat>(values :: IndexedVec<T, n>) -> IndexedVec<T, n> do
  case values do
    Empty -> Empty
    Push(head, tail) -> Push(head, copy(tail))
  end
end
"#,
        );
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        let checked = mesh_typeck::check(&parsed);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let parameters = ["\0mesh_type_0".to_owned(), "\0mesh_index_1".to_owned()];
        let element = Ty::Con(TyCon::new(parameters[0].as_str()));
        let n = NatIndex::Parameter(parameters[1].clone());
        let vector = |element, index| {
            let registry = &checked.type_registry;
            Ty::App(
                Box::new(Ty::Con(registry.sum_type_defs["IndexedVec"].head())),
                vec![element, Ty::Nat(index)],
            )
        };
        let nonempty = vector(element.clone(), NatIndex::Succ(Box::new(n.clone())));
        let any = vector(element.clone(), n.clone());
        let int_vec = vector(Ty::Con(TyCon::new("Int")), n);
        let copy_signature = Ty::fun(vec![any.clone()], any.clone());
        let signatures = [
            (
                "first".into(),
                vec![Ty::fun(vec![nonempty.clone()], element.clone())],
            ),
            ("rest".into(), vec![Ty::fun(vec![nonempty], any.clone())]),
            ("rebuild".into(), vec![Ty::fun(vec![any.clone()], any)]),
            (
                "copy_int".into(),
                vec![Ty::fun(vec![int_vec.clone()], int_vec)],
            ),
            ("copy".into(), vec![copy_signature.clone()]),
        ]
        .into();
        let (module, entries) = mesh_codegen::mir::lower::lower_module_to_mir_with_signatures(
            &parsed,
            &checked,
            "",
            &Default::default(),
            &signatures,
            &[],
            |_| Some(MirSourceId(0)),
        )
        .unwrap();
        let registry = &checked.type_registry;
        let declarations = [NominalDeclaration {
            name: "IndexedVec".into(),
            registry,
        }];
        let mut session = ModelSession::new(std::slice::from_ref(&module), &declarations);
        let (family, constructors) = session.nominal_family("IndexedVec", registry).unwrap();
        let push = &constructors[1].1;
        let mut definitions = String::new();
        for name in ["first", "rest", "rebuild"] {
            let entry = entries
                .iter()
                .find(|entry| entry.source_name == name)
                .unwrap();
            let function = function(&module, &entry.symbol);
            let definition = session
                .parametric_definition(function, entry, registry, &parameters, name)
                .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            assert!(definition.contains("(_mesh_t0 : Type) (_mesh_n1 : Nat)"));
            definitions.push_str(&definition);
        }
        let copy = entries
            .iter()
            .find(|entry| entry.source_name == "copy_int")
            .unwrap();
        let body = session
            .parametric_definition(
                function(&module, &copy.symbol),
                copy,
                registry,
                &parameters[1..],
                "copyInt",
            )
            .unwrap_or_else(|error| panic!("copy_int: {error:?}"));
        assert!(body.contains("termination_by structural _mesh_p0"));
        definitions.push_str(&body);

        let variants: Vec<_> = entries
            .iter()
            .filter(|entry| entry.source_name == "copy")
            .map(|entry| (function(&module, &entry.symbol), entry))
            .collect();
        let (copy, entry) = *variants
            .iter()
            .find(|(_, entry)| entry.signature == copy_signature)
            .unwrap();
        let aliases = crate::verify_parametric::check_specializations(
            (copy, entry),
            &variants,
            &[],
            &parameters,
            registry,
        )
        .unwrap();
        assert!(!aliases.is_empty());
        let error = session
            .parametric_definition(copy, entry, registry, &parameters, "copy")
            .unwrap_err();
        assert!(error.reason.contains("unsupported MIR call"), "{error:?}");
        let body = session
            .parametric_definition_with_recursive_aliases(
                copy,
                entry,
                registry,
                &parameters,
                "copy",
                &aliases,
            )
            .unwrap_or_else(|error| panic!("copy: {error:?}"));
        assert!(body.contains("termination_by structural _mesh_p0"));
        definitions.push_str(&body);

        let (native, native_entries) =
            mesh_codegen::mir::lower::lower_module_to_mir_with_signatures(
                &parsed,
                &checked,
                "",
                &Default::default(),
                &Default::default(),
                &[],
                |_| Some(MirSourceId(0)),
            )
            .unwrap();
        let native_copy = native_entries
            .iter()
            .find(|entry| entry.source_name == "copy_int")
            .unwrap();
        assert!(native_copy.signature.has_type_vars());
        ModelSession::new(std::slice::from_ref(&native), &declarations)
            .validate_indexed_native_layouts(native_copy, registry)
            .unwrap();
        let mut shadowed = native.clone();
        let mut shadow = native
            .sum_types
            .iter()
            .find(|ty| ty.name == "IndexedVec")
            .unwrap()
            .clone();
        shadow.name = "IndexedVec_Int".into();
        shadowed.sum_types.push(shadow);
        // The open indexed types in the checked expressions independently
        // require the exact native layout even without a signature mention.
        let mut layout_probe = native_copy.clone();
        layout_probe.signature = Ty::fun(vec![Ty::int()], Ty::int());
        let error = ModelSession::new(&[shadowed], &declarations)
            .validate_indexed_native_layouts(&layout_probe, registry)
            .unwrap_err();
        assert!(
            error.reason.contains("does not uniquely fall back"),
            "{error:?}"
        );

        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let source = format!(
                "{}{}{definitions}
theorem rigid_first (T : Type) (n : Nat) (head : T) (tail : {family} T n) :
    first T n ({push} head tail) = head := rfl
theorem rigid_rest (T : Type) (n : Nat) (head : T) (tail : {family} T n) :
    rest T n ({push} head tail) = tail := rfl
theorem rigid_rebuild (T : Type) (n : Nat) (values : {family} T n) :
    rebuild T n values = values := by
  cases values with
  | ctor0 => rfl
  | ctor1 head tail => rfl
theorem rigid_copy_int (n : Nat) (values : {family} (BitVec 64) n) :
    copyInt n values = values := by
  induction values with
  | ctor0 => rfl
  | @ctor1 k head tail ih => simp only [copyInt, ih]
theorem rigid_copy (T : Type) (n : Nat) (values : {family} T n) :
    copy T n values = values := by
  induction values with
  | ctor0 => rfl
  | @ctor1 k head tail ih => simp only [copy, ih]
",
                super::super::PRELUDE,
                session.prelude(),
            );
            super::super::check_artifact(
                source,
                &[
                    "rigid_first".into(),
                    "rigid_rest".into(),
                    "rigid_rebuild".into(),
                    "rigid_copy_int".into(),
                    "rigid_copy".into(),
                ],
                super::super::Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn generic_mirror_proves_involution_in_lean_at_every_element_type() {
        let parsed = mesh_parser::parse(
            r#"
type Tree<T> do
  Leaf(T)
  Node(Tree<T>, Tree<T>)
end

fn mirror<T>(tree :: Tree<T>) -> Tree<T> do
  case tree do
    Leaf(value) -> Leaf(value)
    Node(left, right) -> Node(mirror(right), mirror(left))
  end
end
"#,
        );
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        let checked = mesh_typeck::check(&parsed);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let parameters = ["\0mesh_test_T".to_owned()];
        let tree = Ty::App(
            Box::new(Ty::Con(TyCon::new("Tree"))),
            vec![Ty::Con(TyCon::new(&parameters[0]))],
        );
        let signature = Ty::fun(vec![tree.clone()], tree);
        let (module, entries) = mesh_codegen::mir::lower::lower_module_to_mir_with_signatures(
            &parsed,
            &checked,
            "",
            &Default::default(),
            &[("mirror".into(), vec![signature])].into(),
            &[],
            |_| Some(MirSourceId(0)),
        )
        .unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.source_name == "mirror")
            .unwrap();
        let function = function(&module, &entry.symbol);
        let registry = &checked.type_registry;
        let declarations = [NominalDeclaration {
            name: "Tree".into(),
            registry,
        }];
        let mut session = ModelSession::new(std::slice::from_ref(&module), &declarations);
        let (family, constructors) = session.nominal_family("Tree", registry).unwrap();
        assert_eq!(
            constructors,
            [
                ("Leaf".into(), format!("{family}.ctor0")),
                ("Node".into(), format!("{family}.ctor1")),
            ]
        );
        let body = session
            .parametric_definition(function, entry, registry, &parameters, "mirror")
            .unwrap();
        assert!(body.contains("def mirror (_mesh_t0 : Type)"));
        assert!(body.contains("(mirror _mesh_t0 "));
        assert_eq!(session.prelude().matches("inductive ").count(), 1);
        let error = session
            .parametric_definition(function, entry, registry, &["\0unknown".into()], "mirror")
            .unwrap_err();
        assert!(error.reason.contains("undeclared rigid"), "{error:?}");

        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let source = format!(
                "{}{}{body}\n\
theorem mirror_involutive (T : Type) (tree : {family} T) : mirror T (mirror T tree) = tree := by\n  \
induction tree with\n  \
| ctor0 value => rfl\n  \
| ctor1 left right ihLeft ihRight => simp only [mirror, ihLeft, ihRight]\n",
                super::super::PRELUDE,
                session.prelude(),
            );
            super::super::check_artifact(
                source,
                &["mirror_involutive".into()],
                super::super::Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap();
        }
    }

    fn callee_param<'a>(signatures: &'a [LoweredSignature], name: &str) -> &'a Ty {
        let entry = signatures
            .iter()
            .find(|entry| entry.symbol == name)
            .unwrap();
        let Ty::Fun(params, _) = &entry.signature else {
            panic!("expected a checked function")
        };
        &params[0]
    }

    #[test]
    fn nominal_models_reject_missing_ownership_conflicting_layouts_and_wrong_scope() {
        let (mut module, registry, signatures) = checked_program(
            r#"
type Event do
  Ready(List<Bool>)
  Idle
end
fn identity(event :: Event) -> Event do
  event
end
"#,
        );
        let signature = &signatures["identity"];
        let owners = [NominalDeclaration {
            name: "Event".into(),
            registry: &registry,
        }];
        let mut other = registry.clone();
        other.sum_type_defs.get_mut("Event").unwrap().variants[0].fields[0] =
            VariantFieldInfo::Positional(Ty::list(Ty::int()));
        {
            let mut session = ModelSession::new(std::slice::from_ref(&module), &owners);
            let identity = function(&module, "identity");
            session
                .translate(identity, &registry, identity, &registry.clone(), signature)
                .unwrap();
            let error = session
                .definition(identity, signature, &other, "wrongScope")
                .unwrap_err();
            assert!(error
                .reason
                .contains("disagrees with its checked source owner"));

            let mut resource = registry.clone();
            resource.register_resource_type("Event");
            let error = session
                .definition(identity, signature, &resource, "resource")
                .unwrap_err();
            assert!(error.reason.contains("resource-bearing"));

            let mut unknown = ModelSession::new(std::slice::from_ref(&module), &[]);
            let error = unknown
                .definition(identity, signature, &registry, "unknown")
                .unwrap_err();
            assert!(error.reason.contains("missing checked source owner"));
        }
        let duplicate = ["Event", "Event"].map(|name| NominalDeclaration {
            name: name.into(),
            registry: &registry,
        });
        let error = ModelSession::new(std::slice::from_ref(&module), &duplicate)
            .definition(
                function(&module, "identity"),
                signature,
                &registry,
                "ambiguous",
            )
            .unwrap_err();
        assert!(error.reason.contains("ambiguous checked source owners"));

        let clean = module.clone();
        let layout = module
            .sum_types
            .iter_mut()
            .find(|def| def.name == "Event")
            .unwrap();
        layout.variants[1].tag = 0;
        let modules = [clean, module];
        let error = ModelSession::new(&modules, &owners)
            .definition(
                function(&modules[0], "identity"),
                signature,
                &registry,
                "duplicateTag",
            )
            .unwrap_err();
        assert!(error.reason.contains("native MIR layout disagree"));
    }

    #[test]
    fn same_type_is_insufficient_for_a_recursive_argument() {
        let (module, registry, signatures) = checked_program(
            r#"
type Count do
  Zero
  More(Count)
end
fn stuck(count :: Count) -> Int do
  case count do
    Zero -> 0
    More(_) -> stuck(count)
  end
end
"#,
        );
        let owners = [NominalDeclaration {
            name: "Count".into(),
            registry: &registry,
        }];
        let error = ModelSession::new(std::slice::from_ref(&module), &owners)
            .definition(
                function(&module, "stuck"),
                &signatures["stuck"],
                &registry,
                "stuck",
            )
            .unwrap_err();
        assert!(error.reason.contains("must decrease a common parameter"));
    }

    #[test]
    fn a_monomorphic_type_cannot_claim_a_generic_instance_abi() {
        let (module, registry, signatures) = checked_program(
            r#"
struct ParcelBox<T> do
  value :: T
end
struct ParcelBox_Int do
  value :: Int
end
fn unbox(box :: ParcelBox_Int) -> Int do
  box.value
end
"#,
        );
        let owners = ["ParcelBox", "ParcelBox_Int"].map(|name| NominalDeclaration {
            name: name.into(),
            registry: &registry,
        });
        let error = ModelSession::new(std::slice::from_ref(&module), &owners)
            .definition(
                function(&module, "unbox"),
                &signatures["unbox"],
                &registry,
                "unbox",
            )
            .unwrap_err();
        assert!(error
            .reason
            .contains("collides with a generic native ABI name"));
    }

    #[test]
    fn inline_sum_tags_are_safe_but_byte_overlay_payloads_may_be_unaligned() {
        let (module, registry, signatures) = checked_program(
            r#"
type Event do
  Empty
  Value(Int)
end
struct Envelope do
  flag :: Bool
  event :: Event
end
fn nested(envelope :: Envelope) -> Int do
  case envelope do
    Envelope { event: Value(n) } -> n
    Envelope { event: Empty } -> 0
  end
end
fn hasValue(envelope :: Envelope) -> Bool do
  case envelope do
    Envelope { event: Value(_) } -> true
    Envelope { event: Empty } -> false
  end
end
fn projected(envelope :: Envelope) -> Int do
  case envelope.event do
    Value(n) -> n
    Empty -> 0
  end
end
"#,
        );
        let owners = ["Event", "Envelope"].map(|name| NominalDeclaration {
            name: name.into(),
            registry: &registry,
        });
        let mut session = ModelSession::new(std::slice::from_ref(&module), &owners);
        let encode = |session: &mut ModelSession<'_>, name| {
            session.definition(function(&module, name), &signatures[name], &registry, name)
        };
        assert!(encode(&mut session, "nested")
            .unwrap_err()
            .reason
            .contains("no guaranteed alignment"));
        let tagged = encode(&mut session, "hasValue").unwrap();
        let projected = encode(&mut session, "projected").unwrap();
        let event = session
            .constructor(&Ty::Con(TyCon::new("Event")), &registry, "Value")
            .unwrap();
        let envelope = session
            .constructor(&Ty::Con(TyCon::new("Envelope")), &registry, "mk")
            .unwrap();
        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let artifact = format!(
                "{}{}{tagged}{projected}
theorem model_payload : hasValue ({envelope} true ({event} 4#64)) = true ∧
    projected ({envelope} true ({event} 4#64)) = 4#64 := ⟨rfl, rfl⟩",
                super::super::PRELUDE,
                session.prelude(),
            );
            super::super::check_artifact(
                artifact,
                &["model_payload".into()],
                super::super::Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn nested_lists_and_tuples_use_checked_types_and_native_mir() {
        let (pack, signature) = lower(
            r#"
fn pack(packet :: (List<List<Int>>, Bool, List<Int>)) -> List<(Bool, List<List<Int>>)> do
  case packet do
    (rows, flag, extra) -> [(flag, [[], extra] ++ rows)]
  end
end
"#,
        );
        let model = translate(&pack, &pack, &signature).unwrap();
        assert_eq!(
            model.parameter_types,
            ["(List (List (BitVec 64)) × Bool × List (BitVec 64))"]
        );
        let packing = translate_definition(&pack, &signature, "pack").unwrap();
        assert!(packing.contains("([] : List (BitVec 64))"));

        let (copy, signature) = lower(
            r#"
fn copy(packets :: List<(Bool, List<List<Int>>)>) -> List<(Bool, List<List<Int>>)> do
  case packets do
    (flag, rows) :: rest -> [(flag, rows)] ++ copy(rest)
    [] -> []
  end
end
"#,
        );
        let copying = translate_definition(&copy, &signature, "copyPackets").unwrap();
        assert!(copying.contains("termination_by structural _mesh_p0"));

        let (select, signature) = lower(
            r#"
fn select(packet :: (List<List<Int>>, Bool, List<Int>)) -> (Bool, List<Int>, Int) do
  (Tuple.second(packet), Tuple.nth(packet, 2), Tuple.size(packet))
end
"#,
        );
        let selecting = translate_definition(&select, &signature, "select").unwrap();

        if std::env::var_os("MESH_VERIFY_LEAN").is_some() {
            let source = format!(
                "{}{packing}{copying}{selecting}{}",
                super::super::PRELUDE,
                r#"
theorem pack_fields (rows : List (List (BitVec 64))) (flag : Bool) (extra : List (BitVec 64)) :
    pack (rows, flag, extra) = [(flag, [] :: extra :: rows)] := rfl

theorem copy_fields (packets : List (Bool × List (List (BitVec 64)))) :
    copyPackets packets = packets := by
  induction packets with
  | nil => rfl
  | cons head tail ih =>
    cases head with
    | mk flag rows => exact congrArg (List.cons (flag, rows)) ih

theorem select_fields (rows : List (List (BitVec 64))) (flag : Bool) (extra : List (BitVec 64)) :
    select (rows, flag, extra) = (flag, extra, 3#64) := rfl
"#,
            );
            super::super::check_artifact(
                source,
                &[
                    "pack_fields".into(),
                    "copy_fields".into(),
                    "select_fields".into(),
                ],
                super::super::Settings {
                    emit: None,
                    verdict: false,
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn pointer_abi_does_not_unify_distinct_source_types() {
        let (identity, _) = lower("fn identity(values :: List<Int>) -> List<Int> do values end");
        let (_, different) =
            lower("fn different(values :: List<Int>) -> (Int, Bool) do (1, true) end");
        let error = translate_definition(&identity, &different, "badIdentity").unwrap_err();
        assert!(error.reason.contains("expected kernel model type"));
        assert!(error.reason.contains("got List (BitVec 64)"));

        let (opaque, signature) =
            lower("fn opaque(value :: Map<Int, Bool>) -> Map<Int, Bool> do value end");
        let error = translate_definition(&opaque, &signature, "opaque").unwrap_err();
        assert!(error
            .reason
            .contains("unsupported kernel model source type Map<Int, Bool>"));
    }
}
