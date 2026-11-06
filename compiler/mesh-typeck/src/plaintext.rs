//! Copyable source labels, independent of runtime/ABI representation.
//!
//! Each function exports a parameter-dependency summary and effects. Ordinary
//! helpers cannot wash a label away by returning an unlabelled annotation.
//! Unknown boundaries are conservative; trusted sealing is resolved before any
//! spelling-based standard-library rule, so imported lookalikes are not seals.

use crate::{error::TypeError, infer::TypeRegistry, ty::Ty, ImportContext};
use mesh_parser::ast::{
    expr::Expr,
    item::{Block, FnDef, Item, LetBinding},
    AstNode,
};
use mesh_parser::{Parse, SyntaxKind, SyntaxNode};
use rowan::TextRange;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

// Paths are bounded to keep recursive source types and summaries finite. A
// truncated/unknown path falls back to the containing value's dependencies.
const MAX_PROJECTION_DEPTH: usize = 12;
const MAX_SOURCE_SHAPE_NODES: usize = 512;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Projection {
    Field(String),
    /// Symbolic selection/control provenance, distinct from data or a tag.
    /// This is internal flow metadata, not a runtime field.
    PriorControl,
    Tag,
    Variant(String, usize),
    Item,
    Length,
    Tuple(usize),
}
type ShapeMap = BTreeMap<Vec<Projection>, Dependencies>;
/// Shared immutable layouts make aliases and function summaries cheap to copy.
/// Only an actual override or summary refinement copies a shared map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shape(Arc<ShapeMap>);
impl std::ops::Deref for Shape {
    type Target = ShapeMap;
    fn deref(&self) -> &ShapeMap {
        &self.0
    }
}
impl std::ops::DerefMut for Shape {
    fn deref_mut(&mut self) -> &mut ShapeMap {
        Arc::make_mut(&mut self.0)
    }
}
impl FromIterator<(Vec<Projection>, Dependencies)> for Shape {
    fn from_iter<I: IntoIterator<Item = (Vec<Projection>, Dependencies)>>(values: I) -> Self {
        Self(Arc::new(values.into_iter().collect()))
    }
}
impl IntoIterator for Shape {
    type Item = (Vec<Projection>, Dependencies);
    type IntoIter = std::collections::btree_map::IntoIter<Vec<Projection>, Dependencies>;
    fn into_iter(self) -> Self::IntoIter {
        Arc::try_unwrap(self.0)
            .unwrap_or_else(|shared| (*shared).clone())
            .into_iter()
    }
}
impl<'a> IntoIterator for &'a Shape {
    type Item = (&'a Vec<Projection>, &'a Dependencies);
    type IntoIter = std::collections::btree_map::Iter<'a, Vec<Projection>, Dependencies>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a> IntoIterator for &'a mut Shape {
    type Item = (&'a Vec<Projection>, &'a mut Dependencies);
    type IntoIter = std::collections::btree_map::IterMut<'a, Vec<Projection>, Dependencies>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
impl Shape {
    fn into_keys(self) -> impl Iterator<Item = Vec<Projection>> {
        self.into_iter().map(|(key, _)| key)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceType {
    parameters: Vec<String>,
    fields: BTreeMap<String, Ty>,
    variants: BTreeMap<String, Vec<Ty>>,
    alias: Option<Ty>,
    ambiguous: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dependencies {
    pub parameters: BTreeSet<usize>,
    pub projections: BTreeSet<(usize, Vec<Projection>)>,
    pub intrinsic: bool,
}
impl Dependencies {
    fn is_empty(&self) -> bool {
        !self.intrinsic && self.parameters.is_empty() && self.projections.is_empty()
    }
    fn merge(&mut self, other: &Self) {
        // Empty/equal immutable dependency facts are common at structural
        // boundaries. Avoid quadratic prefix scans for these exact unions.
        if other.parameters.is_empty() && other.projections.is_empty() {
            self.intrinsic |= other.intrinsic;
            return;
        }
        if self.parameters.is_empty() && self.projections.is_empty() {
            let intrinsic = self.intrinsic || other.intrinsic;
            *self = other.clone();
            self.intrinsic = intrinsic;
            return;
        }
        if self == other {
            return;
        }
        self.parameters.extend(&other.parameters);
        // A whole argument already includes every field dependency. Keeping
        // those descendants as well makes aggregate forwarding/recursion
        // repeatedly substitute equivalent projection sets quadratically.
        // The same inclusion holds between a projection and its descendants.
        self.projections
            .retain(|(index, _)| !self.parameters.contains(index));
        for (index, path) in &other.projections {
            if self.parameters.contains(index)
                || self
                    .projections
                    .iter()
                    .any(|(prior_index, prior)| prior_index == index && path.starts_with(prior))
            {
                continue;
            }
            self.projections
                .retain(|(prior_index, prior)| prior_index != index || !prior.starts_with(path));
            self.projections.insert((*index, path.clone()));
        }
        self.intrinsic |= other.intrinsic;
    }
    fn union(mut self, other: &Self) -> Self {
        self.merge(other);
        self
    }
    fn substitute(&self, args: &[Value]) -> Self {
        let mut result = Self {
            intrinsic: self.intrinsic,
            ..Self::default()
        };
        for index in &self.parameters {
            if let Some(value) = args.get(*index) {
                result.merge(&value.deps);
            } else {
                // An implicit/piped slot may not appear in the explicit argument
                // list. Retain all actual dependencies without inventing a source
                // label in an otherwise entirely unlabelled program.
                for value in args {
                    result.merge(&value.deps);
                }
            }
        }
        for (index, path) in &self.projections {
            if let Some(value) = args.get(*index) {
                result.merge(&value.dependency_path(path));
            } else {
                for value in args {
                    result.merge(&value.deps);
                }
            }
        }
        result
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaintextSummary {
    pub output: Dependencies,
    pub output_shape: Shape,
    /// Proven possible constructors. None is unknown; an empty set is bottom
    /// only while solving an analyzable local function's recursive component.
    pub output_variants: Option<BTreeSet<String>>,
    pub effects: BTreeMap<String, Dependencies>,
    pub builder_writes: BTreeMap<usize, Dependencies>,
    pub builder_requirements: BTreeMap<usize, Dependencies>,
    pub builder_escapes: BTreeSet<usize>,
}
impl PlaintextSummary {
    fn merge(&mut self, other: Self) {
        match (&mut self.output_variants, other.output_variants) {
            (Some(current), Some(variants)) => current.extend(variants),
            (current, _) => *current = None,
        }
        // Preserve the shared layout when a fixed-point iteration is already
        // stable. An unconditional mutable walk would copy the entire map.
        if self.output_shape != other.output_shape {
            for (path, deps) in &mut self.output_shape {
                deps.merge(shape_dependency(&other.output_shape, &other.output, path));
            }
            for (path, deps) in other.output_shape {
                self.output_shape.entry(path).or_default().merge(&deps);
            }
        }
        self.output.merge(&other.output);
        for (sink, deps) in other.effects {
            self.effects.entry(sink).or_default().merge(&deps);
        }
        for (parameter, deps) in other.builder_writes {
            self.builder_writes
                .entry(parameter)
                .or_default()
                .merge(&deps);
        }
        for (parameter, deps) in other.builder_requirements {
            self.builder_requirements
                .entry(parameter)
                .or_default()
                .merge(&deps);
        }
        self.builder_escapes.extend(other.builder_escapes);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaintextSite {
    pub kind: String,
    pub reason: String,
    pub span: TextRange,
}

#[derive(Default)]
pub struct PlaintextCheck {
    pub functions: FxHashMap<String, PlaintextSummary>,
    pub type_names: FxHashSet<String>,
    pub source_types: BTreeMap<String, SourceType>,
    pub sites: Vec<PlaintextSite>,
    pub expressions: FxHashSet<TextRange>,
    pub errors: Vec<TypeError>,
}

#[derive(Clone, Default)]
struct Value {
    deps: Dependencies,
    shape: Shape,
    shape_control: Dependencies,
    exits: Dependencies,
    variants: Option<BTreeSet<String>>,
    terminated: bool,
    callable: Option<String>,
    actor: Option<ActorOrigin>,
    closure: Option<Box<ClosureContext>>,
    receiver: Option<Box<Value>>,
    declassified: bool,
    builders: BTreeSet<BuilderLocation>,
}
impl Value {
    fn resolved_shape(&self) -> Shape {
        if self.shape_control.is_empty() {
            self.shape.clone()
        } else {
            // Keep selection provenance once, independently from the payload
            // paths. Instantiation restores this marker as shape control;
            // copying it into every descendant is both redundant and costly.
            let mut shape = self.shape.clone();
            shape
                .entry(vec![Projection::PriorControl])
                .or_default()
                .merge(&self.shape_control);
            shape
        }
    }
    fn dependent(deps: Dependencies) -> Self {
        Self {
            deps,
            ..Self::default()
        }
    }
    fn project_path(&self, path: &[Projection]) -> Self {
        if path.is_empty() {
            return self.clone();
        }
        let deps = self.dependency_path(path);
        let shape = self
            .shape
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(path)
                    .filter(|rest| !rest.is_empty())
                    .map(|rest| (rest.to_vec(), value.clone()))
            })
            .collect();
        Self {
            deps,
            shape,
            shape_control: self.prior_control(path),
            exits: self.exits.clone(),
            builders: if matches!(path, [Projection::Variant(_, _)]) {
                self.builders.clone()
            } else {
                BTreeSet::new()
            },
            declassified: self.declassified,
            ..Self::default()
        }
    }
    fn dependency_path(&self, path: &[Projection]) -> Dependencies {
        if let Some((Projection::PriorControl, prefix)) = path.split_last() {
            return self.prior_control(prefix);
        }
        shape_dependency(&self.shape, &self.deps, path)
            .clone()
            .union(&self.prior_control(path))
    }
    fn prior_control(&self, path: &[Projection]) -> Dependencies {
        let mut control = self.shape_control.clone();
        let mut key = path.to_vec();
        key.push(Projection::PriorControl);
        for length in (0..=path.len()).rev() {
            key.truncate(length + 1);
            key[length] = Projection::PriorControl;
            if let Some(deps) = self.shape.get(&key) {
                control.merge(deps);
            } else if length > 0 {
                // Missing structural proof (including the bounded expansion
                // frontier) cannot certify that earlier selection was public.
                control.merge(shape_dependency(&self.shape, &self.deps, &path[..length]));
            }
        }
        control
    }
    fn project(&self, projection: Projection) -> Self {
        self.project_path(&[projection])
    }
    fn put(&mut self, projection: Projection, value: &Self) {
        self.shape
            .insert(vec![projection.clone()], value.deps.clone());
        // An explicit empty marker is also a proof: without it, an unknown
        // or truncated projection must conservatively retain whole-value data.
        self.shape.insert(
            vec![projection.clone(), Projection::PriorControl],
            value.shape_control.clone(),
        );
        for (path, deps) in &value.shape {
            if path.len() < MAX_PROJECTION_DEPTH {
                let mut key = vec![projection.clone()];
                key.extend_from_slice(path);
                self.shape.insert(key, deps.clone());
            }
        }
        self.deps.merge(&value.deps);
        self.exits.merge(&value.exits);
    }
    fn control(&mut self, pc: &Dependencies) {
        self.deps.merge(pc);
        self.shape_control.merge(pc);
    }
    fn alternatives(values: &[Self], pc: &Dependencies) -> Self {
        let mut exits = Dependencies::default();
        for value in values {
            exits.merge(&value.exits);
        }
        let normal = values
            .iter()
            .filter(|value| !value.terminated)
            .collect::<Vec<_>>();
        if let [value] = normal.as_slice() {
            let mut output = (*value).clone();
            output.control(pc);
            output.exits.merge(&exits);
            return output;
        }
        let mut output = Self::dependent(pc.clone());
        output.shape_control = pc.clone();
        output.exits = exits;
        output.terminated = normal.is_empty();
        let paths = normal
            .iter()
            .flat_map(|value| value.shape.keys().cloned())
            .collect::<BTreeSet<_>>();
        for value in &normal {
            output.deps.merge(&value.deps);
            output.shape_control.merge(&value.shape_control);
            output.builders.extend(value.builders.iter().cloned());
        }
        for path in paths {
            let mut deps = pc.clone();
            for value in &normal {
                if let Some(Projection::Variant(variant, _)) = path.first() {
                    if value
                        .variants
                        .as_ref()
                        .is_some_and(|variants| !variants.contains(variant))
                    {
                        continue;
                    }
                }
                deps.merge(&value.dependency_path(&path));
            }
            output.shape.insert(path, deps);
        }
        output.variants = normal
            .iter()
            .try_fold(BTreeSet::new(), |mut variants, value| {
                variants.extend(value.variants.as_ref()?.iter().cloned());
                Some(variants)
            });
        output
    }
}

type Environment = FxHashMap<String, Value>;

fn shape_dependency<'a>(
    shape: &'a Shape,
    whole: &'a Dependencies,
    path: &[Projection],
) -> &'a Dependencies {
    (1..=path.len())
        .rev()
        .find_map(|length| shape.get(&path[..length]))
        .unwrap_or(whole)
}

fn substitute_type(ty: &Ty, substitutions: &BTreeMap<String, Ty>) -> Ty {
    match ty {
        Ty::Con(con) if substitutions.contains_key(&con.name) => substitutions[&con.name].clone(),
        _ => ty.map_parts(|part| substitute_type(part, substitutions)),
    }
}

#[derive(Clone)]
struct ActorOrigin {
    name: String,
    state: Vec<Dependencies>,
}

#[derive(Clone)]
struct ClosureContext {
    syntax: mesh_parser::ast::expr::ClosureExpr,
    captured: Environment,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BuilderLocation {
    Parameter(usize),
    Local(u32, u32),
    Unknown,
}

#[derive(Default)]
struct BuilderContents {
    deps: Dependencies,
    escaped: bool,
}

struct ShapeInstantiation {
    source: Shape,
    arguments: Vec<(Dependencies, Shape, Dependencies)>,
    output: Shape,
}
impl ShapeInstantiation {
    fn matches(&self, source: &Shape, arguments: &[Value]) -> bool {
        &self.source == source
            && self.arguments.len() == arguments.len()
            && self.arguments.iter().zip(arguments).all(|(prior, value)| {
                prior.0 == value.deps && prior.1 == value.shape && prior.2 == value.shape_control
            })
    }
}

struct Checker<'a> {
    types: &'a FxHashMap<TextRange, Ty>,
    registry: &'a TypeRegistry,
    type_names: FxHashSet<String>,
    source_types: BTreeMap<String, SourceType>,
    summaries: FxHashMap<String, PlaintextSummary>,
    imported_namespaces: FxHashSet<String>,
    imported_function_names: FxHashSet<String>,
    locals: FxHashSet<String>,
    local_namespaces: FxHashSet<String>,
    actors: FxHashMap<String, mesh_parser::ast::item::ActorDef>,
    proven_actors: FxHashSet<String>,
    mailbox: Option<Value>,
    globals: Environment,
    namespace: String,
    builders: BTreeMap<BuilderLocation, BuilderContents>,
    shape_instantiations: FxHashMap<TextRange, ShapeInstantiation>,
    summary_reads: FxHashSet<String>,
    overloaded: &'a FxHashSet<String>,
    call_targets: &'a FxHashMap<TextRange, String>,
    current: PlaintextSummary,
    returned: Vec<Value>,
    errors: Vec<TypeError>,
    sites: BTreeMap<(u32, u32, String), PlaintextSite>,
    expressions: FxHashSet<TextRange>,
    emit: bool,
}

pub(crate) fn check(
    parse: &Parse,
    types: &FxHashMap<TextRange, Ty>,
    registry: &TypeRegistry,
    imports: &ImportContext,
    overloaded: &FxHashSet<String>,
    call_targets: &FxHashMap<TextRange, String>,
) -> PlaintextCheck {
    let functions: Vec<_> = parse
        .syntax()
        .descendants()
        .filter_map(FnDef::cast)
        .collect();
    let mut checker = Checker {
        types,
        registry,
        type_names: FxHashSet::default(),
        source_types: BTreeMap::new(),
        summaries: FxHashMap::default(),
        imported_namespaces: FxHashSet::default(),
        imported_function_names: FxHashSet::default(),
        locals: FxHashSet::default(),
        local_namespaces: FxHashSet::default(),
        actors: parse
            .syntax()
            .descendants()
            .filter_map(mesh_parser::ast::item::ActorDef::cast)
            .filter_map(|actor| {
                actor
                    .name()
                    .and_then(|name| name.text())
                    .map(|name| (name, actor))
            })
            .collect(),
        proven_actors: FxHashSet::default(),
        mailbox: None,
        globals: Environment::default(),
        namespace: String::new(),
        builders: BTreeMap::new(),
        shape_instantiations: FxHashMap::default(),
        summary_reads: FxHashSet::default(),
        overloaded,
        call_targets,
        current: PlaintextSummary::default(),
        returned: Vec::new(),
        errors: Vec::new(),
        sites: BTreeMap::new(),
        expressions: FxHashSet::default(),
        emit: false,
    };
    for item in parse.tree().items() {
        let (path, selected) = match item {
            Item::ImportDecl(value) => (value.module_path(), None),
            Item::FromImportDecl(value) => (
                value.module_path(),
                Some(
                    value
                        .import_list()
                        .iter()
                        .flat_map(|list| list.names())
                        .filter_map(|name| name.text())
                        .collect::<Vec<_>>(),
                ),
            ),
            _ => continue,
        };
        let segments = path.map(|path| path.segments()).unwrap_or_default();
        let Some(exports) = imports.exports_for_path(&segments.join(".")) else {
            continue;
        };
        let Some(namespace) = segments.last() else {
            continue;
        };
        checker
            .type_names
            .extend(exports.plaintext_type_names.iter().cloned());
        for (name, definition) in &exports.plaintext_source_types {
            checker.add_source_type(name.clone(), definition.clone());
            if selected.is_none()
                && !name.contains('.')
                && (exports.struct_defs.contains_key(name)
                    || exports.sum_type_defs.contains_key(name)
                    || exports.type_aliases.contains_key(name))
            {
                checker.add_source_type(format!("{namespace}.{name}"), definition.clone());
            }
        }
        checker.imported_namespaces.insert(namespace.clone());
        for (name, summary) in &exports.plaintext_functions {
            if let Some(actor) = name.strip_prefix("@actor.") {
                if selected
                    .as_ref()
                    .is_none_or(|selected| selected.iter().any(|name| name == actor))
                {
                    let actor = if selected.is_some() {
                        actor.to_string()
                    } else {
                        format!("{namespace}.{actor}")
                    };
                    checker.proven_actors.insert(actor.clone());
                    checker
                        .summaries
                        .insert(format!("@actor.{actor}"), summary.clone());
                }
                continue;
            }
            if let Some(selected) = &selected {
                if selected
                    .iter()
                    .any(|item| crate::ownership::source_function_name(name) == *item)
                {
                    checker.summaries.insert(name.clone(), summary.clone());
                    checker.imported_function_names.insert(name.clone());
                }
            } else {
                checker
                    .summaries
                    .insert(format!("{namespace}.{name}"), summary.clone());
            }
        }
        // Imports from an older/unknown embedding context have no effect proof.
        for name in exports.functions.keys() {
            if selected.as_ref().is_none_or(|names| {
                names
                    .iter()
                    .any(|item| crate::ownership::source_function_name(name) == *item)
            }) {
                checker
                    .imported_function_names
                    .insert(if selected.is_some() {
                        name.clone()
                    } else {
                        format!("{namespace}.{name}")
                    });
            }
        }
    }
    checker.discover_source_types(parse);
    checker.discover_labelled_types(parse);
    checker.local_namespaces.extend(
        parse
            .syntax()
            .descendants()
            .filter_map(mesh_parser::ast::item::ModuleDef::cast)
            .filter_map(|module| module.name().and_then(|name| name.text())),
    );
    for function in &functions {
        if let Some(name) = checker.function_name(function) {
            checker.locals.insert(name.clone());
            let output_type = function
                .return_type()
                .and_then(|ann| crate::infer::raw_annotation_type(&ann))
                .or_else(|| {
                    types
                        .get(&function.syntax().text_range())
                        .and_then(|ty| match ty {
                            Ty::Fun(_, output) => Some(output.as_ref().clone()),
                            _ => None,
                        })
                });
            // Establish structural paths at bottom before solving dependencies.
            // Otherwise a forward call's temporarily absent field proof would
            // permanently add a containing record's private dependency.
            let output_shape = output_type
                .map(|ty| {
                    checker
                        .source_value(&ty, None, &[], false)
                        .shape
                        .into_keys()
                        .map(|path| (path, Dependencies::default()))
                        .collect()
                })
                .unwrap_or_default();
            checker
                .summaries
                .entry(name)
                .or_insert_with(|| PlaintextSummary {
                    output_shape,
                    output_variants: (function.native_decl().is_none()
                        && (function.body().is_some() || function.expr_body().is_some()))
                    .then(BTreeSet::new),
                    ..PlaintextSummary::default()
                });
        }
    }
    let actors = checker.actors.values().cloned().collect::<Vec<_>>();
    checker.proven_actors.extend(checker.actors.keys().cloned());
    for actor in &actors {
        checker
            .summaries
            .entry(format!(
                "@actor.{}",
                actor
                    .name()
                    .and_then(|name| name.text())
                    .unwrap_or_default()
            ))
            .or_default();
    }
    // Finite monotone dependency/effect domain, including recursive helpers.
    // Revisit actual summary readers rather than copying every unrelated
    // aggregate through every round of a recursive component's fixed point.
    let mut first = true;
    let mut changed = FxHashSet::default();
    let mut reads: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    let has_global_initializers = parse.syntax().descendants().any(|node| {
        node.kind() == SyntaxKind::LET_BINDING
            && !node.ancestors().any(|parent| {
                matches!(
                    parent.kind(),
                    SyntaxKind::FN_DEF | SyntaxKind::ACTOR_DEF | SyntaxKind::CLOSURE_EXPR
                )
            })
    });
    loop {
        // Global initializers can capture closures or aliases. Keep the full
        // module iteration for those modules; only immutable summary-to-summary
        // dependencies use the selective worklist.
        let globals_changed = first || has_global_initializers;
        let prior_globals = checker
            .globals
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    (
                        value.deps.clone(),
                        value.shape.clone(),
                        value.shape_control.clone(),
                        value.variants.clone(),
                    ),
                )
            })
            .collect::<FxHashMap<_, _>>();
        checker.summary_reads.clear();
        checker.global_scope(parse);
        let global_values_changed = checker.globals.iter().any(|(name, value)| {
            prior_globals.get(name)
                != Some(&(
                    value.deps.clone(),
                    value.shape.clone(),
                    value.shape_control.clone(),
                    value.variants.clone(),
                ))
        });
        let mut next = FxHashSet::default();
        for function in &functions {
            let Some(name) = checker.function_name(function) else {
                continue;
            };
            if !globals_changed
                && reads
                    .get(&name)
                    .is_some_and(|prior| prior.is_disjoint(&changed))
            {
                continue;
            }
            checker.summary_reads.clear();
            let summary = checker.function(function);
            reads.insert(name.clone(), std::mem::take(&mut checker.summary_reads));
            let prior = checker.summaries.entry(name.clone()).or_default();
            let old = prior.clone();
            prior.merge(summary);
            if *prior != old {
                next.insert(name);
            }
        }
        for actor in &actors {
            let name = format!(
                "@actor.{}",
                actor
                    .name()
                    .and_then(|name| name.text())
                    .unwrap_or_default()
            );
            if !globals_changed
                && reads
                    .get(&name)
                    .is_some_and(|prior| prior.is_disjoint(&changed))
            {
                continue;
            }
            checker.summary_reads.clear();
            let summary = checker.actor(actor);
            reads.insert(name.clone(), std::mem::take(&mut checker.summary_reads));
            let prior = checker.summaries.entry(name.clone()).or_default();
            let old = prior.clone();
            prior.merge(summary);
            if *prior != old {
                next.insert(name);
            }
        }
        if next.is_empty() && !global_values_changed {
            break;
        }
        changed = next;
        first = false;
    }
    checker.emit = true;
    checker.global_scope(parse);
    for function in &functions {
        checker.function(function);
    }
    for actor in &actors {
        checker.actor(actor);
    }
    PlaintextCheck {
        functions: checker.summaries,
        type_names: checker.type_names,
        source_types: checker.source_types,
        sites: checker.sites.into_values().collect(),
        expressions: checker.expressions,
        errors: checker.errors,
    }
}

impl Checker<'_> {
    fn add_source_type(&mut self, name: String, definition: SourceType) {
        match self.source_types.get_mut(&name) {
            Some(prior) if *prior != definition => prior.ambiguous = true,
            Some(_) => (),
            None => {
                self.source_types.insert(name, definition);
            }
        }
    }

    fn discover_source_types(&mut self, parse: &Parse) {
        for item in parse.syntax().descendants().filter_map(Item::cast) {
            let (name, node, mut definition) = match item {
                Item::StructDef(value) => {
                    let fields = value
                        .fields()
                        .filter_map(|field| {
                            Some((
                                field.name()?.text()?,
                                crate::infer::raw_annotation_type(&field.type_annotation()?)?,
                            ))
                        })
                        .collect();
                    (
                        value.name().and_then(|name| name.text()),
                        value.syntax().clone(),
                        SourceType {
                            fields,
                            ..SourceType::default()
                        },
                    )
                }
                Item::SumTypeDef(value) => {
                    let variants = value
                        .variants()
                        .filter_map(|variant| {
                            let fields = variant
                                .positional_types()
                                .chain(variant.fields().filter_map(|field| field.type_annotation()))
                                .filter_map(|ann| crate::infer::raw_annotation_type(&ann))
                                .collect();
                            Some((variant.name()?.text().to_string(), fields))
                        })
                        .collect();
                    (
                        value.name().and_then(|name| name.text()),
                        value.syntax().clone(),
                        SourceType {
                            variants,
                            ..SourceType::default()
                        },
                    )
                }
                Item::TypeAliasDef(value) => (
                    value.name().and_then(|name| name.text()),
                    value.syntax().clone(),
                    SourceType {
                        alias: crate::infer::type_after_eq(value.syntax()),
                        ..SourceType::default()
                    },
                ),
                _ => continue,
            };
            definition.parameters = node
                .children()
                .find(|child| child.kind() == SyntaxKind::GENERIC_PARAM_LIST)
                .map(|params| {
                    params
                        .children_with_tokens()
                        .filter_map(|element| element.into_token())
                        .filter(|token| token.kind() == SyntaxKind::IDENT)
                        .map(|token| token.text().to_string())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(name) = name {
                self.add_source_type(name, definition);
            }
        }
    }

    fn source_value(
        &self,
        ty: &Ty,
        parameter: Option<usize>,
        path: &[Projection],
        forced: bool,
    ) -> Value {
        let mut budget = MAX_SOURCE_SHAPE_NODES;
        self.source_value_at(ty, parameter, path, forced, 0, &mut budget)
    }

    fn empty_list_value(&self, span: TextRange, pc: &Dependencies) -> Value {
        let mut value = Value::dependent(pc.clone());
        if let Some(ty) = self.types.get(&span) {
            // An empty list contains no data-derived items. Establish its
            // inferred structural paths at bottom, so a later typed annotation
            // or append cannot invent a dependency for an unrelated field.
            // The surrounding control and any explicit whole-value label still
            // cover every projection in the usual way.
            value.shape = self
                .source_value(ty, None, &[], false)
                .shape
                .into_keys()
                .map(|path| (path, pc.clone()))
                .collect();
        }
        value.put(Projection::Length, &Value::dependent(pc.clone()));
        value.put(Projection::Item, &Value::dependent(pc.clone()));
        value
    }

    fn source_value_at(
        &self,
        ty: &Ty,
        parameter: Option<usize>,
        path: &[Projection],
        forced: bool,
        depth: usize,
        budget: &mut usize,
    ) -> Value {
        let mut deps = Dependencies::default();
        if let Some(index) = parameter {
            if path.is_empty() {
                deps.parameters.insert(index);
            } else {
                deps.projections.insert((index, path.to_vec()));
            }
        }
        deps.intrinsic = forced;
        let mut value = Value::dependent(deps);
        value.shape_control.intrinsic = forced;
        if let Some(index) = parameter {
            let mut control_path = path.to_vec();
            control_path.push(Projection::PriorControl);
            value
                .shape_control
                .projections
                .insert((index, control_path));
        }
        if depth >= MAX_PROJECTION_DEPTH || *budget == 0 {
            value.deps.intrinsic |= self.labelled_type(ty);
            return value;
        }
        *budget -= 1;
        let args = match ty {
            Ty::App(_, args) => args.as_slice(),
            _ => &[],
        };
        let name = ty.con_name().unwrap_or("");
        if name == "Plaintext" {
            return args
                .first()
                .map(|inner| self.source_value_at(inner, parameter, path, true, depth + 1, budget))
                .unwrap_or_else(|| {
                    Value::dependent(Dependencies {
                        intrinsic: true,
                        ..Dependencies::default()
                    })
                });
        }
        let mut children = Vec::new();
        match name {
            "Bytes" => children.push((Projection::Length, Ty::int())),
            "Result" if args.len() == 2 => {
                children.push((Projection::Tag, Ty::int()));
                children.push((Projection::Variant("Ok".into(), 0), args[0].clone()));
                children.push((Projection::Variant("Err".into(), 0), args[1].clone()));
            }
            "Option" if args.len() == 1 => {
                children.push((Projection::Tag, Ty::int()));
                children.push((Projection::Variant("Some".into(), 0), args[0].clone()));
            }
            "List" if args.len() == 1 => {
                children.push((Projection::Length, Ty::int()));
                children.push((Projection::Item, args[0].clone()));
            }
            _ => {
                if let Ty::Tuple(items) = ty {
                    children.extend(
                        items
                            .iter()
                            .enumerate()
                            .map(|(index, ty)| (Projection::Tuple(index), ty.clone())),
                    );
                } else if let Some(definition) = self
                    .source_types
                    .get(name)
                    .filter(|definition| !definition.ambiguous)
                {
                    let substitutions = definition
                        .parameters
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect::<BTreeMap<_, _>>();
                    if let Some(alias) = &definition.alias {
                        // An alias does not add a projection segment; use the
                        // depth guard separately to stop recursive aliases.
                        if alias.con_name() != Some(name) {
                            return self.source_value_at(
                                &substitute_type(alias, &substitutions),
                                parameter,
                                path,
                                forced,
                                depth + 1,
                                budget,
                            );
                        }
                    }
                    children.extend(definition.fields.iter().map(|(field, ty)| {
                        (
                            Projection::Field(field.clone()),
                            substitute_type(ty, &substitutions),
                        )
                    }));
                    if !definition.variants.is_empty() {
                        children.push((Projection::Tag, Ty::int()));
                        for (variant, fields) in &definition.variants {
                            children.extend(fields.iter().enumerate().map(|(index, ty)| {
                                (
                                    Projection::Variant(variant.clone(), index),
                                    substitute_type(ty, &substitutions),
                                )
                            }));
                        }
                    }
                } else {
                    // Unmodelled generic containers still contain their
                    // labelled elements. Losing projection precision must
                    // never drop a nested source label altogether.
                    value.deps.intrinsic |= self.labelled_type(ty);
                }
            }
        }
        for (part, ty) in children {
            let mut next = path.to_vec();
            next.push(part.clone());
            let child = self.source_value_at(&ty, parameter, &next, forced, depth + 1, budget);
            value.put(part, &child);
        }
        value
    }

    fn annotate(&self, value: &mut Value, annotation: &mesh_parser::ast::item::TypeAnnotation) {
        if let Some(ty) = crate::infer::raw_annotation_type(annotation) {
            let template = self.source_value(&ty, None, &[], false);
            if template.deps.intrinsic {
                value.declassified = false;
            }
            value.deps.intrinsic |= template.deps.intrinsic;
            value.shape_control.merge(&template.shape_control);
            for (path, sensitivity) in template.shape {
                if sensitivity.intrinsic {
                    let fallback = value.deps.clone();
                    value.shape.entry(path).or_insert(fallback).intrinsic = true;
                }
            }
        }
    }

    fn function_name(&self, function: &FnDef) -> Option<String> {
        let name = function.name()?.text()?;
        let name = if self.overloaded.contains(&name) {
            format!(
                "{name}__{}",
                function
                    .param_list()
                    .map_or(0, |params| params.params().count())
            )
        } else {
            name
        };
        let mut namespaces = function
            .syntax()
            .ancestors()
            .filter_map(mesh_parser::ast::item::ModuleDef::cast)
            .filter_map(|module| module.name().and_then(|name| name.text()))
            .collect::<Vec<_>>();
        namespaces.reverse();
        namespaces.push(name);
        Some(namespaces.join("."))
    }

    fn annotation_labelled(&self, node: &SyntaxNode) -> bool {
        node.descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .any(|token| {
                token.kind() == SyntaxKind::IDENT
                    && (token.text() == "Plaintext" || self.type_names.contains(token.text()))
            })
    }

    fn discover_labelled_types(&mut self, parse: &Parse) {
        loop {
            let before = self.type_names.len();
            for item in parse.syntax().descendants().filter_map(Item::cast) {
                let (name, node) = match item {
                    Item::StructDef(value) => (
                        value.name().and_then(|name| name.text()),
                        value.syntax().clone(),
                    ),
                    Item::SumTypeDef(value) => (
                        value.name().and_then(|name| name.text()),
                        value.syntax().clone(),
                    ),
                    Item::TypeAliasDef(value) => (
                        value.name().and_then(|name| name.text()),
                        value.syntax().clone(),
                    ),
                    _ => continue,
                };
                if self.annotation_labelled(&node) {
                    if let Some(name) = name {
                        self.type_names.insert(name);
                    }
                }
            }
            // Propagate through imported/reachable aggregate layouts too.
            for (name, definition) in &self.registry.struct_defs {
                if definition
                    .fields
                    .iter()
                    .any(|(_, ty)| self.labelled_type(ty))
                {
                    self.type_names.insert(name.clone());
                }
            }
            if self.type_names.len() == before {
                break;
            }
        }
    }

    fn labelled_type(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Fun(..) => false, // a signature is not a captured value
            Ty::Con(con) => con.name == "Plaintext" || self.type_names.contains(&con.name),
            _ => ty.parts().any(|part| self.labelled_type(part)),
        }
    }

    fn error(&mut self, span: TextRange, sink: &str, reason: &str) {
        if self.emit {
            self.errors.push(TypeError::PlaintextViolation {
                sink: sink.into(),
                reason: reason.into(),
                span,
            });
        }
    }

    fn effect(&mut self, sink: &str, deps: Dependencies, span: TextRange) {
        self.current
            .effects
            .entry(sink.into())
            .or_default()
            .merge(&deps);
        if deps.intrinsic {
            self.error(span, sink, "plaintext or control flow derived from it requires a seal, reviewed display export, or explicit declassification");
        }
    }

    fn require_builder(&mut self, value: &Value, deps: Dependencies, span: TextRange) {
        if value.builders.is_empty() {
            self.effect("unproven builder write", deps, span);
            return;
        }
        for location in &value.builders {
            match location {
                BuilderLocation::Parameter(index) => {
                    self.current
                        .builder_requirements
                        .entry(*index)
                        .or_default()
                        .merge(&deps);
                    if self
                        .builders
                        .get(location)
                        .is_some_and(|contents| contents.escaped)
                    {
                        self.effect("escaped builder write", deps.clone(), span);
                    }
                }
                BuilderLocation::Local(..)
                    if self
                        .builders
                        .get(location)
                        .is_some_and(|contents| !contents.escaped) =>
                {
                    ()
                }
                _ => self.effect("unproven builder write", deps.clone(), span),
            }
        }
    }

    fn write_builder(&mut self, value: &Value, deps: Dependencies, span: TextRange) {
        self.require_builder(value, deps.clone(), span);
        for location in &value.builders {
            if let BuilderLocation::Parameter(index) = location {
                self.current
                    .builder_writes
                    .entry(*index)
                    .or_default()
                    .merge(&deps);
            }
            self.builders
                .entry(location.clone())
                .or_default()
                .deps
                .merge(&deps);
        }
    }

    fn escape_builder(&mut self, value: &Value) {
        for location in &value.builders {
            if let BuilderLocation::Parameter(index) = location {
                self.current.builder_escapes.insert(*index);
            }
            self.builders.entry(location.clone()).or_default().escaped = true;
        }
    }

    fn bind_pattern(&self, node: &SyntaxNode, value: &Value, env: &mut Environment) {
        if let Some(pattern) = mesh_parser::ast::pat::Pattern::cast(node.clone()) {
            self.bind_projected_pattern(&pattern, value, env);
        }
    }

    /// A bodyless arm is the value rebuilt from its matched pattern (E0056),
    /// just as lowering reconstructs that constructor. It is not a return of
    /// the complete scrutinee, including payloads of unreachable variants.
    fn rebuilt_pattern_value(
        &self,
        pattern: &mesh_parser::ast::pat::Pattern,
        input: &Value,
        pc: &Dependencies,
    ) -> Value {
        use mesh_parser::ast::pat::Pattern;
        let (variant, fields) = match pattern {
            Pattern::Ident(ident) => {
                let name = ident
                    .name()
                    .map(|name| name.text().to_string())
                    .unwrap_or_default();
                if !name.starts_with(char::is_uppercase) {
                    let mut output = input.clone();
                    output.control(pc);
                    return output;
                }
                (name, Vec::new())
            }
            Pattern::Constructor(constructor) => {
                let name = constructor
                    .variant_name()
                    .map(|name| name.text().to_string())
                    .unwrap_or_default();
                let fields = constructor
                    .fields()
                    .enumerate()
                    .map(|(index, pattern)| {
                        self.rebuilt_pattern_value(
                            &pattern,
                            &input.project(Projection::Variant(name.clone(), index)),
                            pc,
                        )
                    })
                    .collect();
                (name, fields)
            }
            Pattern::Literal(_) => return Value::dependent(pc.clone()),
            // Other patterns are not values and are rejected by E0056. Stay
            // conservative while checking an incomplete or erroneous program.
            _ => {
                let mut output = input.clone();
                output.control(pc);
                return output;
            }
        };
        let mut output = Value::dependent(pc.clone());
        output.control(pc);
        output.variants = Some(BTreeSet::from([variant.clone()]));
        output.put(Projection::Tag, &Value::dependent(pc.clone()));
        for (index, field) in fields.into_iter().enumerate() {
            output.put(Projection::Variant(variant.clone(), index), &field);
            output.builders.extend(field.builders.iter().cloned());
        }
        output.exits.merge(&input.exits);
        output
    }

    fn bind_projected_pattern(
        &self,
        pattern: &mesh_parser::ast::pat::Pattern,
        value: &Value,
        env: &mut Environment,
    ) {
        use mesh_parser::ast::pat::Pattern;
        match pattern {
            Pattern::Ident(ident) => {
                if let Some(name) = ident
                    .name()
                    .filter(|name| !name.text().starts_with(char::is_uppercase))
                {
                    env.insert(name.text().into(), value.clone());
                }
            }
            Pattern::Constructor(constructor) => {
                let name = constructor
                    .variant_name()
                    .map(|name| name.text().to_string())
                    .unwrap_or_default();
                for (index, pattern) in constructor.fields().enumerate() {
                    self.bind_projected_pattern(
                        &pattern,
                        &value.project(Projection::Variant(name.clone(), index)),
                        env,
                    );
                }
            }
            Pattern::Struct(record) => {
                for field in record.fields() {
                    if let (Some(name), Some(pattern)) = (field.name(), field.pattern()) {
                        self.bind_projected_pattern(
                            &pattern,
                            &value.project(Projection::Field(name.text().into())),
                            env,
                        );
                    }
                }
            }
            Pattern::Tuple(tuple) => {
                for (index, pattern) in tuple.patterns().enumerate() {
                    self.bind_projected_pattern(
                        &pattern,
                        &value.project(Projection::Tuple(index)),
                        env,
                    );
                }
            }
            Pattern::List(list) => {
                for pattern in list.patterns() {
                    self.bind_projected_pattern(&pattern, &value.project(Projection::Item), env);
                }
            }
            Pattern::Cons(cons) => {
                if let Some(pattern) = cons.head() {
                    self.bind_projected_pattern(&pattern, &value.project(Projection::Item), env);
                }
                if let Some(pattern) = cons.tail() {
                    self.bind_projected_pattern(&pattern, value, env);
                }
            }
            Pattern::As(alias) => {
                if let Some(pattern) = alias.pattern() {
                    self.bind_projected_pattern(&pattern, value, env);
                }
                if let Some(name) = alias.binding_name() {
                    env.insert(name.text().into(), value.clone());
                }
            }
            Pattern::Or(or) => {
                let mut alternatives: FxHashMap<String, Vec<Value>> = FxHashMap::default();
                for pattern in or.alternatives() {
                    let mut scope = Environment::default();
                    self.bind_projected_pattern(&pattern, value, &mut scope);
                    for (name, value) in scope {
                        alternatives.entry(name).or_default().push(value);
                    }
                }
                for (name, values) in alternatives {
                    env.insert(name, Value::alternatives(&values, &Dependencies::default()));
                }
            }
            _ => (),
        }
    }

    fn pattern_control(
        &self,
        pattern: &mesh_parser::ast::pat::Pattern,
        value: &Value,
    ) -> Dependencies {
        use mesh_parser::ast::pat::Pattern;
        let mut deps = Dependencies::default();
        match pattern {
            Pattern::Literal(_) => deps.merge(&value.deps),
            Pattern::Ident(ident)
                if ident
                    .name()
                    .is_some_and(|name| name.text().starts_with(char::is_uppercase)) =>
            {
                deps.merge(&value.project(Projection::Tag).deps)
            }
            Pattern::Constructor(constructor) => {
                deps.merge(&value.project(Projection::Tag).deps);
                let name = constructor
                    .variant_name()
                    .map(|name| name.text().to_string())
                    .unwrap_or_default();
                for (index, pattern) in constructor.fields().enumerate() {
                    deps.merge(&self.pattern_control(
                        &pattern,
                        &value.project(Projection::Variant(name.clone(), index)),
                    ));
                }
            }
            Pattern::Struct(record) => {
                for field in record.fields() {
                    if let (Some(name), Some(pattern)) = (field.name(), field.pattern()) {
                        deps.merge(&self.pattern_control(
                            &pattern,
                            &value.project(Projection::Field(name.text().into())),
                        ));
                    }
                }
            }
            Pattern::Tuple(tuple) => {
                for (index, pattern) in tuple.patterns().enumerate() {
                    deps.merge(
                        &self.pattern_control(&pattern, &value.project(Projection::Tuple(index))),
                    );
                }
            }
            Pattern::List(list) => {
                deps.merge(&value.project(Projection::Length).deps);
                for pattern in list.patterns() {
                    deps.merge(&self.pattern_control(&pattern, &value.project(Projection::Item)));
                }
            }
            Pattern::Cons(cons) => {
                deps.merge(&value.project(Projection::Length).deps);
                if let Some(pattern) = cons.head() {
                    deps.merge(&self.pattern_control(&pattern, &value.project(Projection::Item)));
                }
                if let Some(pattern) = cons.tail() {
                    deps.merge(&self.pattern_control(&pattern, value));
                }
            }
            Pattern::As(alias) => {
                if let Some(pattern) = alias.pattern() {
                    deps.merge(&self.pattern_control(&pattern, value));
                }
            }
            Pattern::Or(or) => {
                for pattern in or.alternatives() {
                    deps.merge(&self.pattern_control(&pattern, value));
                }
            }
            _ => (),
        }
        deps
    }

    fn function(&mut self, function: &FnDef) -> PlaintextSummary {
        self.current = PlaintextSummary::default();
        self.returned.clear();
        self.builders.clear();
        self.mailbox = None;
        self.namespace = self
            .function_name(function)
            .and_then(|name| {
                name.rsplit_once('.')
                    .map(|(prefix, _)| format!("{prefix}."))
            })
            .unwrap_or_default();
        let mut env = self.global_environment();
        let mut pc = Dependencies::default();
        if let Some(params) = function.param_list() {
            for (index, param) in params.params().enumerate() {
                let parameter_type =
                    self.types
                        .get(&function.syntax().text_range())
                        .and_then(|ty| match ty {
                            Ty::Fun(parameters, _) => parameters.get(index),
                            _ => None,
                        });
                let mut value = param
                    .type_annotation()
                    .and_then(|ann| crate::infer::raw_annotation_type(&ann))
                    .or_else(|| parameter_type.cloned())
                    .map(|ty| self.source_value(&ty, Some(index), &[], false))
                    .unwrap_or_else(|| {
                        Value::dependent(Dependencies {
                            parameters: BTreeSet::from([index]),
                            ..Dependencies::default()
                        })
                    });
                if parameter_type.is_some_and(contains_builder)
                    || param.type_annotation().is_some_and(|annotation| {
                        annotation
                            .syntax()
                            .descendants_with_tokens()
                            .any(|element| {
                                element
                                    .as_token()
                                    .is_some_and(|token| token.text() == "BytesBuilder")
                            })
                    })
                {
                    let location = BuilderLocation::Parameter(index);
                    value.builders.insert(location.clone());
                    self.builders.insert(
                        location,
                        BuilderContents {
                            deps: value.deps.clone(),
                            escaped: false,
                        },
                    );
                }
                if let Some(name) = param.name() {
                    env.insert(name.text().into(), value.clone());
                }
                if let Some(pattern) = param.pattern() {
                    pc.merge(&self.pattern_control(&pattern, &value));
                    self.bind_pattern(pattern.syntax(), &value, &mut env);
                }
            }
        }
        if let Some(guard) = function.guard() {
            let flow = self.node(guard.syntax(), &mut env, &pc);
            pc.merge(&flow.deps);
        }
        let output = if function.native_decl().is_some() {
            for value in env.values() {
                self.escape_builder(value);
            }
            for (index, param) in function
                .param_list()
                .into_iter()
                .flat_map(|list| list.params().collect::<Vec<_>>())
                .enumerate()
            {
                self.effect(
                    "native ABI",
                    Dependencies {
                        parameters: BTreeSet::from([index]),
                        intrinsic: param
                            .type_annotation()
                            .is_some_and(|ann| self.annotation_labelled(ann.syntax())),
                        ..Dependencies::default()
                    },
                    function.syntax().text_range(),
                );
            }
            Value::default()
        } else if let Some(body) = function.body() {
            self.block(&body, &mut env, &pc)
        } else if let Some(body) = function.expr_body() {
            self.expr(&body, &mut env, &pc)
        } else {
            Value::default()
        };
        self.returned.push(output);
        let mut output = Value::alternatives(&self.returned, &Dependencies::default());
        if let Some(annotation) = function.return_type() {
            self.annotate(&mut output, &annotation);
        }
        self.current.output.merge(&output.deps);
        self.current.output_shape = output.resolved_shape();
        self.current.output_variants = output.variants;
        let display = function.display_decl().and_then(|declaration| {
            let reason = declaration.reason().and_then(|reason| review_reason(&reason));
            if function.export_decl().is_none() || function.native_decl().is_some() || reason.is_none() {
                self.error(declaration.syntax().text_range(), "display", "a display boundary requires a bodyful @export and a nonempty literal review reason");
                return None;
            }
            if self.emit { self.sites.insert((u32::from(declaration.syntax().text_range().start()),u32::from(declaration.syntax().text_range().end()),"display".into()),
                PlaintextSite { kind: "display".into(), reason: reason.clone().unwrap(), span: declaration.syntax().text_range() }); }
            reason
        });
        if function.export_decl().is_some() && display.is_none() {
            self.effect(
                "native export",
                self.current.output.clone(),
                function.syntax().text_range(),
            );
        }
        std::mem::take(&mut self.current)
    }

    fn block(&mut self, block: &Block, env: &mut Environment, pc: &Dependencies) -> Value {
        self.sequence(block.syntax(), env, pc)
    }

    fn sequence(&mut self, syntax: &SyntaxNode, env: &mut Environment, pc: &Dependencies) -> Value {
        let mut control = pc.clone();
        let mut exits = Dependencies::default();
        let mut output = Value::dependent(control.clone());
        for node in syntax.children() {
            if let Some(module) = mesh_parser::ast::item::ModuleDef::cast(node.clone()) {
                let prior = self.namespace.clone();
                self.namespace.push_str(
                    &module
                        .name()
                        .and_then(|name| name.text())
                        .unwrap_or_default(),
                );
                self.namespace.push('.');
                let mut scope = env.clone();
                if let Some(body) = module.syntax().children().find_map(Block::cast) {
                    self.sequence(body.syntax(), &mut scope, &control);
                    for binding in body.syntax().children().filter_map(LetBinding::cast) {
                        if let Some(name) = binding.name().and_then(|name| name.text()) {
                            if let Some(value) = scope.get(&name) {
                                env.insert(format!("{}{name}", self.namespace), value.clone());
                            }
                        }
                    }
                    for (name, value) in scope {
                        if name.starts_with(&self.namespace) {
                            env.insert(name, value);
                        }
                    }
                }
                self.namespace = prior;
            } else if let Some(binding) = LetBinding::cast(node.clone()) {
                let initializer = binding.initializer();
                let mut value = initializer
                    .clone()
                    .map(|expr| self.expr(&expr, env, &control))
                    .unwrap_or_default();
                control.merge(&value.exits);
                exits.merge(&value.exits);
                if let Some(annotation) = binding.type_annotation() {
                    self.annotate(&mut value, &annotation);
                }
                if let Some(name) = binding.name().and_then(|name| name.text()) {
                    env.insert(name, value.clone());
                }
                if let Some(pattern) = binding.pattern() {
                    control.merge(&self.pattern_control(&pattern, &value));
                    self.bind_pattern(pattern.syntax(), &value, env);
                }
                output = Value::dependent(control.clone());
            } else if let Some(expr) = Expr::cast(node) {
                output = self.expr(&expr, env, &control);
                control.merge(&output.exits);
                exits.merge(&output.exits);
            }
        }
        output.exits.merge(&exits);
        output
    }

    fn global_scope(&mut self, parse: &Parse) {
        self.current = PlaintextSummary::default();
        self.builders.clear();
        self.mailbox = None;
        self.namespace.clear();
        let mut env = self.globals.clone();
        self.sequence(&parse.syntax(), &mut env, &Dependencies::default());
        self.globals = env;
    }

    fn global_environment(&self) -> Environment {
        let mut environment = self.globals.clone();
        for (name, value) in &self.globals {
            if let Some(local) = name
                .strip_prefix(&self.namespace)
                .filter(|name| !name.contains('.'))
            {
                environment.insert(local.into(), value.clone());
            }
        }
        environment
    }

    fn actor(&mut self, actor: &mesh_parser::ast::item::ActorDef) -> PlaintextSummary {
        self.current = PlaintextSummary::default();
        self.builders.clear();
        let mut env = self.globals.clone();
        let mut arity = 0;
        for (index, param) in actor
            .param_list()
            .into_iter()
            .flat_map(|list| list.params().collect::<Vec<_>>())
            .enumerate()
        {
            arity = index + 1;
            let value = Value::dependent(Dependencies {
                parameters: BTreeSet::from([index]),
                intrinsic: param
                    .type_annotation()
                    .is_some_and(|annotation| self.annotation_labelled(annotation.syntax())),
                ..Dependencies::default()
            });
            if let Some(name) = param.name() {
                env.insert(name.text().into(), value.clone());
            }
            if let Some(pattern) = param.pattern() {
                self.bind_pattern(pattern.syntax(), &value, &mut env);
            }
        }
        self.mailbox = Some(Value::dependent(Dependencies {
            parameters: BTreeSet::from([arity]),
            intrinsic: false,
            ..Dependencies::default()
        }));
        if let Some(body) = actor.body() {
            self.block(&body, &mut env, &Dependencies::default());
        }
        self.current.output = Dependencies::default();
        self.mailbox = None;
        std::mem::take(&mut self.current)
    }

    fn actor_effects(
        &mut self,
        origin: &ActorOrigin,
        message: Value,
        pc: &Dependencies,
        span: TextRange,
    ) {
        let mut arguments = origin
            .state
            .iter()
            .cloned()
            .map(Value::dependent)
            .collect::<Vec<_>>();
        arguments.push(message);
        let name = format!("@actor.{}", origin.name);
        self.summary_reads.insert(name.clone());
        if let Some(summary) = self.summaries.get(&name).cloned() {
            for (sink, deps) in summary.effects {
                self.effect(&sink, deps.substitute(&arguments).union(pc), span);
            }
        } else {
            self.effect("unknown actor", join(&arguments).union(pc), span);
        }
    }

    fn node(&mut self, node: &SyntaxNode, env: &mut Environment, pc: &Dependencies) -> Value {
        if let Some(expr) = Expr::cast(node.clone()) {
            return self.expr(&expr, env, pc);
        }
        let children = node.children().collect::<Vec<_>>();
        if let [child] = children.as_slice() {
            return self.node(child, env, pc);
        }
        let mut value = Value::dependent(pc.clone());
        let mut control = pc.clone();
        for child in children {
            let child = self.node(&child, env, &control);
            value.deps.merge(&child.deps);
            control.merge(&child.exits);
            value.exits.merge(&child.exits);
            value.builders.extend(child.builders);
        }
        value
    }

    fn expr(&mut self, expr: &Expr, env: &mut Environment, pc: &Dependencies) -> Value {
        let value = match expr {
            Expr::NameRef(_) => self.expr_name(expr, env, pc),
            Expr::CallExpr(_) => self.expr_call(expr, env, pc),
            Expr::FieldAccess(_) => self.expr_field(expr, env, pc),
            _ => self.expr_complex(expr, env, pc),
        };
        self.finish_expr(expr, value, pc)
    }

    fn finish_expr(&mut self, expr: &Expr, mut value: Value, pc: &Dependencies) -> Value {
        value.control(pc);
        if let Some((name, ty)) = value
            .callable
            .as_deref()
            .zip(self.types.get(&expr.syntax().text_range()))
        {
            let variant = name.rsplit('.').next().unwrap_or(name);
            let nullary = (ty.con_name() == Some("Option") && variant == "None")
                || ty
                    .con_name()
                    .and_then(|name| self.source_types.get(name))
                    .filter(|definition| !definition.ambiguous)
                    .and_then(|definition| definition.variants.get(variant))
                    .is_some_and(Vec::is_empty);
            if nullary {
                value
                    .shape
                    .entry(vec![Projection::Tag])
                    .or_insert_with(|| value.deps.clone());
                value.variants = Some(BTreeSet::from([variant.into()]));
            }
        }
        for location in &value.builders {
            if let Some(contents) = self.builders.get(location) {
                value.deps.merge(&contents.deps);
            }
        }
        let empty_constructor = match expr {
            Expr::CallExpr(call) => {
                matches!(call.callee(),Some(Expr::NameRef(name)) if name.text().as_deref() == Some("Err"))
            }
            _ => false,
        };
        if value.builders.is_empty()
            && !empty_constructor
            && !matches!(expr, Expr::ReturnExpr(..))
            && self
                .types
                .get(&expr.syntax().text_range())
                .is_some_and(contains_builder)
        {
            value.builders.insert(BuilderLocation::Unknown);
        }
        if !value.declassified
            && self
                .types
                .get(&expr.syntax().text_range())
                .is_some_and(|ty| self.labelled_type(ty))
        {
            value.deps.intrinsic = true;
        }
        if self.emit && value.deps.intrinsic {
            self.expressions.insert(expr.syntax().text_range());
        }
        value
    }

    fn expr_name(&mut self, expr: &Expr, env: &mut Environment, _pc: &Dependencies) -> Value {
        let Expr::NameRef(name) = expr else {
            unreachable!()
        };
        let name = name.text().unwrap_or_default();
        if matches!(name.as_str(), "declassify" | "declassify_discriminant")
            && !env.contains_key(&name)
            && !self.locals.contains(&name)
            && !self.imported_function_names.contains(&name)
        {
            let direct = expr
                .syntax()
                .parent()
                .is_some_and(|parent| match Expr::cast(parent) {
                    Some(Expr::CallExpr(call)) => call
                        .callee()
                        .is_some_and(|callee| callee.syntax() == expr.syntax()),
                    _ => false,
                });
            if !direct {
                self.error(expr.syntax().text_range(), &name, "call the builtin directly with a literal review reason; it cannot escape as a function value");
            }
        }
        env.get(&name).cloned().unwrap_or_else(|| Value {
            callable: Some(name),
            ..Value::default()
        })
    }

    fn expr_call(&mut self, expr: &Expr, env: &mut Environment, pc: &Dependencies) -> Value {
        let Expr::CallExpr(call) = expr else {
            unreachable!()
        };
        let callee = call
            .callee()
            .map(|expr| self.expr(&expr, env, pc))
            .unwrap_or_default();
        let mut args = call.args();
        let mut evaluation = pc.clone().union(&callee.exits);
        let mut values = self.arguments(&args, env, &mut evaluation);
        let control = if let Some(receiver) = &callee.receiver {
            values.insert(0, receiver.as_ref().clone());
            if let Some(Expr::FieldAccess(field)) = call.callee() {
                if let Some(base) = field.base() {
                    args.insert(0, base);
                }
            }
            evaluation
        } else {
            evaluation.union(&callee.deps)
        };
        if let Some(closure) = &callee.closure {
            // Captured data is read from the checked closure environment. It
            // is not a private choice of which function to invoke; branches
            // and unknown function values deliberately lose this proof.
            let mut closure_control = pc.clone().union(&callee.exits);
            for value in &values {
                closure_control.merge(&value.exits);
            }
            let mut output = self.invoke_closure(closure, &values, &closure_control);
            for value in &values {
                output.exits.merge(&value.exits);
            }
            output.exits.merge(&callee.exits);
            return output;
        }
        let name = self
            .call_targets
            .get(&call.syntax().text_range())
            .cloned()
            .or(callee.callable);
        let mut output = self.call(
            name.as_deref(),
            &args,
            &values,
            &control,
            call.syntax().text_range(),
        );
        output.exits.merge(&callee.exits);
        output
    }

    fn expr_field(&mut self, expr: &Expr, env: &mut Environment, pc: &Dependencies) -> Value {
        let Expr::FieldAccess(field) = expr else {
            unreachable!()
        };
        let parts = field
            .syntax()
            .children()
            .filter_map(Expr::cast)
            .collect::<Vec<_>>();
        let receiver = parts
            .first()
            .map(|expr| self.expr(expr, env, pc))
            .unwrap_or_default();
        let direct = parts.first().and_then(|expr| {
            if let Expr::NameRef(name) = expr {
                name.text()
            } else {
                None
            }
        });
        let callable = direct
            .filter(|name| !env.contains_key(name))
            .zip(field.field())
            .map(|(base, field)| format!("{base}.{}", field.text()));
        if callable.is_none() {
            let module = parts
                .first()
                .and_then(|expression| self.types.get(&expression.syntax().text_range()))
                .and_then(pure_receiver_module);
            if let Some((module, method)) = module.zip(field.field()) {
                return Value {
                    deps: receiver.deps.clone(),
                    callable: Some(format!("{module}.{}", method.text())),
                    receiver: Some(Box::new(receiver)),
                    ..Value::default()
                };
            }
        }
        if callable.is_none() {
            field
                .field()
                .map(|field| receiver.project(Projection::Field(field.text().into())))
                .unwrap_or(receiver)
        } else {
            Value {
                callable,
                ..Value::default()
            }
        }
    }

    fn expr_complex(&mut self, expr: &Expr, env: &mut Environment, pc: &Dependencies) -> Value {
        match expr {
            Expr::BinaryExpr(binary)
                if matches!(
                    binary.op().map(|token| token.kind()),
                    Some(SyntaxKind::PLUS_PLUS | SyntaxKind::DIAMOND)
                ) && self
                    .types
                    .get(&expr.syntax().text_range())
                    .is_some_and(|ty| ty.con_name() == Some("List")) =>
            {
                let arguments = binary
                    .lhs()
                    .into_iter()
                    .chain(binary.rhs())
                    .collect::<Vec<_>>();
                let mut control = pc.clone();
                let values = self.arguments(&arguments, env, &mut control);
                let left = values.first().cloned().unwrap_or_default();
                let right = values.get(1).cloned().unwrap_or_default();
                let mut length = left.project(Projection::Length);
                length.deps.merge(&right.project(Projection::Length).deps);
                let items = [
                    left.project(Projection::Item),
                    right.project(Projection::Item),
                ];
                let mut output = Value::dependent(control.clone());
                output.put(Projection::Length, &length);
                output.put(Projection::Item, &Value::alternatives(&items, &control));
                output.control(&control);
                output
            }
            Expr::StructLiteral(record) => {
                let mut value = Value::dependent(pc.clone());
                let mut control = pc.clone();
                for field in record.fields() {
                    if let (Some(name), Some(expression)) =
                        (field.name().and_then(|name| name.text()), field.value())
                    {
                        let child = self.expr(&expression, env, &control);
                        control.merge(&child.exits);
                        value.put(Projection::Field(name), &child);
                    }
                }
                value
            }
            Expr::StructUpdate(record) => {
                let mut value = record
                    .base_expr()
                    .map(|expr| self.expr(&expr, env, pc))
                    .unwrap_or_default();
                let mut control = pc.clone().union(&value.exits);
                for field in record.override_fields() {
                    if let (Some(name), Some(expression)) =
                        (field.name().and_then(|name| name.text()), field.value())
                    {
                        let part = Projection::Field(name);
                        value.shape.retain(|path, _| path.first() != Some(&part));
                        let child = self.expr(&expression, env, &control);
                        control.merge(&child.exits);
                        value.put(part, &child);
                    }
                }
                value
            }
            Expr::TupleExpr(tuple) => {
                let mut value = Value::dependent(pc.clone());
                let mut control = pc.clone();
                for (index, expression) in tuple.elements().enumerate() {
                    let child = self.expr(&expression, env, &control);
                    control.merge(&child.exits);
                    value.put(Projection::Tuple(index), &child);
                }
                value
            }
            Expr::ListLiteral(list) => {
                if list.elements().next().is_none() {
                    return self.empty_list_value(expr.syntax().text_range(), pc);
                }
                let mut value = Value::dependent(pc.clone());
                value.put(Projection::Length, &Value::dependent(pc.clone()));
                let elements =
                    self.arguments(&list.elements().collect::<Vec<_>>(), env, &mut pc.clone());
                value.put(Projection::Item, &Value::alternatives(&elements, pc));
                value
            }
            Expr::Block(block) => self.block(block, &mut env.clone(), pc),
            Expr::IfExpr(branch) => {
                let condition = branch
                    .condition()
                    .map(|expr| self.expr(&expr, env, pc))
                    .unwrap_or_default();
                let control = pc.clone().union(&condition.deps).union(&condition.exits);
                let mut values = Vec::new();
                if let Some(block) = branch.then_branch() {
                    let branch = self.block(&block, &mut env.clone(), &control);
                    values.push(branch);
                }
                if let Some(other) = branch.else_branch() {
                    let branch = self.node(other.syntax(), &mut env.clone(), &control);
                    values.push(branch);
                }
                let mut result = Value::alternatives(&values, &control);
                if has_control_exit(expr) {
                    result.exits.merge(&condition.deps);
                }
                result
            }
            Expr::CaseExpr(case) => {
                let input = case
                    .scrutinee()
                    .map(|expr| self.expr(&expr, env, pc))
                    .unwrap_or_default();
                let mut control = pc.clone().union(&input.exits);
                for arm in case.arms() {
                    if let Some(pattern) = arm.pattern() {
                        control.merge(&self.pattern_control(&pattern, &input));
                    }
                }
                // A failed guard determines whether any following arm runs.
                // Check all guards before assigning a common branch control.
                for arm in case.arms() {
                    let mut scope = env.clone();
                    if let Some(pattern) = arm.pattern() {
                        self.bind_pattern(pattern.syntax(), &input, &mut scope);
                    }
                    if let Some(guard) = arm.guard() {
                        let guard = self.expr(&guard, &mut scope, pc);
                        control.merge(&guard.deps);
                        control.merge(&guard.exits);
                    }
                }
                let mut values = Vec::new();
                for arm in case.arms() {
                    let mut branch = env.clone();
                    if let Some(pattern) = arm.pattern() {
                        self.bind_pattern(pattern.syntax(), &input, &mut branch);
                    }
                    let mut branch_pc = control.clone();
                    if let Some(guard) = arm.guard() {
                        branch_pc.merge(&self.expr(&guard, &mut branch, &control).deps);
                    }
                    if let Some(body) = arm.body() {
                        let branch = self.expr(&body, &mut branch, &branch_pc);
                        values.push(branch);
                    } else if let Some(pattern) = arm.pattern() {
                        values.push(self.rebuilt_pattern_value(&pattern, &input, &branch_pc));
                    } else {
                        values.push(input.clone());
                    }
                }
                let mut output = Value::alternatives(&values, &control);
                if has_control_exit(expr) {
                    output.exits.merge(&control);
                }
                output.exits.merge(&input.exits);
                output
            }
            Expr::ForInExpr(loop_expr) => {
                let input = loop_expr
                    .iterable()
                    .map(|expr| self.expr(&expr, env, pc))
                    .unwrap_or_default();
                let mut scope = env.clone();
                let element = input.project(Projection::Item);
                let mut control = pc
                    .clone()
                    .union(&input.project(Projection::Length).deps)
                    .union(&input.exits);
                if let Some(name) = loop_expr.binding_name().and_then(|name| name.text()) {
                    scope.insert(name, element.clone());
                }
                if let Some(pattern) = loop_expr.pattern() {
                    control.merge(&self.pattern_control(&pattern, &element));
                    self.bind_pattern(pattern.syntax(), &element, &mut scope);
                }
                if let Some(filter) = loop_expr.filter() {
                    let filter = self.expr(&filter, &mut scope, &control.clone());
                    control.merge(&filter.deps);
                    control.merge(&filter.exits);
                }
                let item = loop_expr
                    .body()
                    .map(|body| self.block(&body, &mut scope, &control))
                    .unwrap_or_default();
                // A comprehension produces a list, rather than returning its
                // body item's shape. Its count depends on iteration/filter
                // control and conditional exits, not on the item payload.
                let length = control.clone().union(&item.exits);
                let mut output = Value::dependent(pc.clone());
                output.put(Projection::Length, &Value::dependent(length));
                output.put(Projection::Item, &item);
                output.exits.merge(&input.exits);
                if has_control_exit(expr) {
                    output.exits.merge(&control);
                }
                output
            }
            Expr::WhileExpr(loop_expr) => {
                let condition = loop_expr
                    .condition()
                    .map(|expr| self.expr(&expr, env, pc))
                    .unwrap_or_default();
                let control = pc.clone().union(&condition.deps).union(&condition.exits);
                let mut result = loop_expr
                    .body()
                    .map(|body| self.block(&body, &mut env.clone(), &control))
                    .unwrap_or_default();
                result.deps.merge(&control);
                result.exits.merge(&condition.exits);
                result
            }
            Expr::ReturnExpr(ret) => {
                let mut result = ret
                    .value()
                    .map(|expr| self.expr(&expr, env, pc))
                    .unwrap_or_else(|| Value::dependent(pc.clone()));
                self.current.output.merge(&result.deps);
                self.returned.push(result.clone());
                result.exits.merge(pc);
                result.terminated = true;
                result.builders.clear();
                result
            }
            Expr::TryExpr(value) => {
                let input = value
                    .operand()
                    .map(|operand| self.expr(&operand, env, pc))
                    .unwrap_or_default();
                let tag = input.project(Projection::Tag).deps;
                let mut output = input.project(Projection::Variant("Ok".into(), 0));
                output.control(&tag);
                output.builders = input.builders.clone();
                output.exits = input.exits.clone().union(&tag);
                let mut error = Value::dependent(tag.clone());
                error.variants = Some(BTreeSet::from(["Err".into()]));
                error.put(Projection::Tag, &Value::dependent(tag));
                error.put(
                    Projection::Variant("Err".into(), 0),
                    &input.project(Projection::Variant("Err".into(), 0)),
                );
                self.returned.push(error);
                output
            }
            Expr::SendExpr(send) => {
                let args = send
                    .arg_list()
                    .into_iter()
                    .flat_map(|list| list.args().collect::<Vec<_>>())
                    .collect::<Vec<_>>();
                let mut control = pc.clone();
                let values = self.arguments(&args, env, &mut control);
                let deps = join(&values).union(&control);
                if let Some(origin) = values.first().and_then(|value| value.actor.as_ref()) {
                    self.actor_effects(
                        origin,
                        values.get(1).cloned().unwrap_or_default(),
                        &control,
                        send.syntax().text_range(),
                    );
                } else {
                    self.effect("actor message", deps.clone(), send.syntax().text_range());
                }
                let mut value = Value::dependent(deps);
                for argument in values {
                    value.exits.merge(&argument.exits);
                }
                value
            }
            Expr::SpawnExpr(spawn) => {
                let args = spawn
                    .arg_list()
                    .into_iter()
                    .flat_map(|list| list.args().collect::<Vec<_>>())
                    .collect::<Vec<_>>();
                let mut control = pc.clone();
                let values = self.arguments(&args, env, &mut control);
                let deps = join(&values).union(&control);
                let mut output = if let Some(name) = values
                    .first()
                    .and_then(|value| value.callable.as_ref())
                    .filter(|name| self.proven_actors.contains(*name))
                {
                    let origin = ActorOrigin {
                        name: name.clone(),
                        state: values
                            .iter()
                            .skip(1)
                            .map(|value| value.deps.clone())
                            .collect(),
                    };
                    self.actor_effects(
                        &origin,
                        Value::default(),
                        &control,
                        spawn.syntax().text_range(),
                    );
                    Value {
                        actor: Some(origin),
                        deps,
                        ..Value::default()
                    }
                } else {
                    self.effect("actor spawn", deps.clone(), spawn.syntax().text_range());
                    Value::dependent(deps)
                };
                for argument in values {
                    output.exits.merge(&argument.exits);
                }
                output
            }
            Expr::ReceiveExpr(receive) => {
                let input = self.mailbox.clone().unwrap_or_default();
                let mut output = Value::dependent(pc.clone());
                for arm in receive.arms() {
                    let mut scope = env.clone();
                    if let Some(pattern) = arm.pattern() {
                        self.bind_pattern(pattern.syntax(), &input, &mut scope);
                    }
                    let mut control = pc.clone().union(&input.deps);
                    if let Some(guard) = arm.guard() {
                        control.merge(&self.expr(&guard, &mut scope, &control.clone()).deps);
                    }
                    if let Some(body) = arm.body() {
                        output
                            .deps
                            .merge(&self.expr(&body, &mut scope, &control).deps);
                    }
                }
                if let Some(timeout) = receive.after_clause() {
                    let duration = timeout
                        .timeout()
                        .map(|value| self.expr(&value, env, pc))
                        .unwrap_or_default();
                    let control = pc.clone().union(&duration.deps);
                    if let Some(body) = timeout.body() {
                        output
                            .deps
                            .merge(&self.expr(&body, &mut env.clone(), &control).deps);
                    }
                }
                output
            }
            Expr::ClosureExpr(closure) => {
                // Captures are conservatively tainted; a closure cannot carry
                // plaintext into an unknown/native callback or another actor.
                let mut scope = env.clone();
                let mut parameters = FxHashSet::default();
                let mut body_control = pc.clone();
                if let Some(params) = closure.param_list() {
                    for (index, param) in params.params().enumerate() {
                        let value = param
                            .type_annotation()
                            .and_then(|ann| crate::infer::raw_annotation_type(&ann))
                            .or_else(|| {
                                self.types
                                    .get(&closure.syntax().text_range())
                                    .and_then(|ty| match ty {
                                        Ty::Fun(parameters, _) => parameters.get(index).cloned(),
                                        _ => None,
                                    })
                            })
                            .map(|ty| self.source_value(&ty, None, &[], false))
                            .unwrap_or_default();
                        if let Some(name) = param.name() {
                            parameters.insert(name.text().to_string());
                            scope.insert(name.text().into(), value.clone());
                        }
                        if let Some(pattern) = param.pattern() {
                            parameters.extend(
                                pattern.binders().iter().map(|name| name.text().to_string()),
                            );
                            body_control.merge(&self.pattern_control(&pattern, &value));
                            self.bind_pattern(pattern.syntax(), &value, &mut scope);
                        }
                    }
                }
                let mut captured = pc.clone();
                let mut captures = Environment::default();
                for reference in closure
                    .syntax()
                    .descendants()
                    .filter_map(mesh_parser::ast::expr::NameRef::cast)
                {
                    if let Some(name) = reference.text() {
                        if parameters.contains(&name) {
                            continue;
                        }
                        if let Some(value) = env.get(&name) {
                            captured.merge(&value.deps);
                            captures.insert(name, value.clone());
                        }
                    }
                }
                let prior = std::mem::take(&mut self.current.output);
                let prior_returns = std::mem::take(&mut self.returned);
                if let Some(body) = closure.body() {
                    self.block(&body, &mut scope, &body_control);
                }
                self.current.output = prior;
                self.returned = prior_returns;
                Value {
                    deps: captured,
                    closure: Some(Box::new(ClosureContext {
                        syntax: closure.clone(),
                        captured: captures,
                    })),
                    ..Value::default()
                }
            }
            Expr::JsonExpr(json) => {
                let value = self.children(json.syntax(), env, pc);
                self.effect(
                    "JSON serialization",
                    value.deps.clone(),
                    json.syntax().text_range(),
                );
                value
            }
            Expr::PipeExpr(pipe) => {
                let lhs = pipe.lhs();
                let left = lhs
                    .as_ref()
                    .map(|expr| self.expr(expr, env, pc))
                    .unwrap_or_default();
                let mut control = pc.clone().union(&left.exits);
                let rhs = pipe.rhs();
                if let Some(Expr::CallExpr(call)) = rhs {
                    let callee = call
                        .callee()
                        .map(|expr| self.expr(&expr, env, &control))
                        .unwrap_or_default();
                    let mut args = call.args();
                    let mut values = vec![left];
                    values.extend(self.arguments(&args, env, &mut control));
                    if let Some(lhs) = lhs {
                        args.insert(0, lhs);
                    }
                    self.call(
                        callee.callable.as_deref(),
                        &args,
                        &values,
                        &control.union(&callee.deps).union(&callee.exits),
                        pipe.syntax().text_range(),
                    )
                } else {
                    let callee = rhs
                        .map(|expr| self.expr(&expr, env, &control))
                        .unwrap_or_default();
                    self.call(
                        callee.callable.as_deref(),
                        &lhs.into_iter().collect::<Vec<_>>(),
                        &[left],
                        &control.union(&callee.deps).union(&callee.exits),
                        pipe.syntax().text_range(),
                    )
                }
            }
            Expr::SlotPipeExpr(pipe) => {
                let lhs = pipe.lhs();
                let left = lhs
                    .as_ref()
                    .map(|expr| self.expr(expr, env, pc))
                    .unwrap_or_default();
                let mut control = pc.clone().union(&left.exits);
                if let Some(Expr::CallExpr(call)) = pipe.rhs() {
                    let callee = call
                        .callee()
                        .map(|expr| self.expr(&expr, env, &control))
                        .unwrap_or_default();
                    let mut args = call.args();
                    let mut values = self.arguments(&args, env, &mut control);
                    let position = pipe.slot().unwrap_or(1).saturating_sub(1) as usize;
                    if position <= values.len() {
                        values.insert(position, left);
                        if let Some(lhs) = lhs {
                            args.insert(position, lhs);
                        }
                    } else {
                        self.effect(
                            "invalid slot-pipe boundary",
                            left.deps,
                            pipe.syntax().text_range(),
                        );
                    }
                    self.call(
                        callee.callable.as_deref(),
                        &args,
                        &values,
                        &control.union(&callee.deps).union(&callee.exits),
                        pipe.syntax().text_range(),
                    )
                } else {
                    self.effect(
                        "unknown slot-pipe boundary",
                        left.deps.clone(),
                        pipe.syntax().text_range(),
                    );
                    left
                }
            }
            _ => self.children(expr.syntax(), env, pc),
        }
    }

    fn children(&mut self, node: &SyntaxNode, env: &mut Environment, pc: &Dependencies) -> Value {
        let mut result = Value::dependent(pc.clone());
        let mut control = pc.clone();
        for child in node.children() {
            let child = self.node(&child, env, &control);
            result.deps.merge(&child.deps);
            control.merge(&child.exits);
            result.exits.merge(&child.exits);
            result.builders.extend(child.builders);
        }
        result
    }

    fn arguments(
        &mut self,
        args: &[Expr],
        env: &mut Environment,
        control: &mut Dependencies,
    ) -> Vec<Value> {
        args.iter()
            .map(|expr| {
                let value = self.expr(expr, env, control);
                control.merge(&value.exits);
                value
            })
            .collect()
    }

    fn call(
        &mut self,
        name: Option<&str>,
        syntax_args: &[Expr],
        args: &[Value],
        pc: &Dependencies,
        span: TextRange,
    ) -> Value {
        let mut control = pc.clone();
        for argument in args {
            control.merge(&argument.exits);
        }
        let mut output = self.call_inner(name, syntax_args, args, &control, span);
        output.control(&control);
        for argument in args {
            output.exits.merge(&argument.exits);
        }
        output
    }

    fn call_inner(
        &mut self,
        name: Option<&str>,
        syntax_args: &[Expr],
        args: &[Value],
        pc: &Dependencies,
        span: TextRange,
    ) -> Value {
        let all = join(args).union(pc);
        let Some(name) = name else {
            self.effect("unknown function value", all.clone(), span);
            return Value::dependent(all);
        };
        let qualified = format!("{}{name}", self.namespace);
        let name = if !self.namespace.is_empty() && self.summaries.contains_key(&qualified) {
            qualified.as_str()
        } else {
            name
        };
        self.summary_reads.insert(name.into());
        if self.actors.contains_key(name) {
            self.actor_effects(
                &ActorOrigin {
                    name: name.into(),
                    state: args.iter().map(|value| value.deps.clone()).collect(),
                },
                self.mailbox.clone().unwrap_or_default(),
                pc,
                span,
            );
            return Value::dependent(all);
        }
        if let Some(summary) = self.summaries.get(name).cloned() {
            for (index, deps) in summary.builder_requirements {
                let value = args.get(index).cloned().unwrap_or_default();
                self.require_builder(&value, deps.substitute(args).union(pc), span);
            }
            for (index, deps) in summary.builder_writes {
                let value = args.get(index).cloned().unwrap_or_default();
                self.write_builder(&value, deps.substitute(args).union(pc), span);
            }
            for index in summary.builder_escapes {
                if let Some(value) = args.get(index) {
                    self.escape_builder(value);
                }
            }
            for (sink, deps) in summary.effects {
                self.effect(&sink, deps.substitute(args).union(pc), span);
            }
            let mut value = Value::dependent(summary.output.substitute(args).union(pc));
            value.shape_control = pc.clone();
            value.shape = match self.shape_instantiations.get(&span) {
                Some(prior) if prior.matches(&summary.output_shape, args) => prior.output.clone(),
                _ => {
                    let mut substitutions = BTreeMap::new();
                    let output: Shape = summary
                        .output_shape
                        .iter()
                        .map(|(path, deps)| {
                            let resolved = substitutions
                                .entry(deps)
                                .or_insert_with(|| deps.substitute(args));
                            (path.clone(), resolved.clone())
                        })
                        .collect();
                    self.shape_instantiations.insert(
                        span,
                        ShapeInstantiation {
                            source: summary.output_shape,
                            arguments: args
                                .iter()
                                .map(|value| {
                                    (
                                        value.deps.clone(),
                                        value.shape.clone(),
                                        value.shape_control.clone(),
                                    )
                                })
                                .collect(),
                            output: output.clone(),
                        },
                    );
                    output
                }
            };
            // Constructor facts belong to the current summary, independently
            // of whether the dependency-only shape instantiation was cached.
            value.variants = summary.output_variants;
            if let Some(control) = value.shape.get(&vec![Projection::PriorControl]) {
                value.shape_control.merge(control);
            }
            value.exits = args.iter().fold(Dependencies::default(), |deps, value| {
                deps.union(&value.exits)
            });
            return value;
        }
        let base = name.split('.').next().unwrap_or(name);
        let external = self.imported_namespaces.contains(base)
            || self.local_namespaces.contains(base)
            || self.imported_function_names.contains(name)
            || self.locals.contains(name);
        if !external && matches!(name, "List.new" | "list_new") {
            return self.empty_list_value(span, pc);
        }
        if !external
            && matches!(
                name,
                "Crypto.sha256" | "crypto_sha256" | "Crypto.sha512" | "crypto_sha512"
            )
        {
            // These genuine total byte APIs hash arbitrary valid Mesh Bytes
            // into fixed [u8; 32]/[u8; 64] provider outputs. Only the extent is
            // public: digest bits, metadata uses, and private caller control
            // retain their dependencies. No fallible Result tag is cleared.
            let mut output = Value::dependent(all);
            output.put(Projection::Length, &Value::dependent(pc.clone()));
            return output;
        }
        if !external && matches!(name, "Bytes.slice" | "bytes_slice") {
            // The genuine slice validates only extent/start/count before
            // copying. Preserve private byte values, range and caller control;
            // the successful length is exactly the requested count. A caller
            // selecting that payload must still observe the fallible tag.
            let mut bounds = pc.clone();
            if let Some(input) = args.first() {
                bounds.merge(&input.project(Projection::Length).deps);
            }
            for argument in args.iter().skip(1) {
                bounds.merge(&argument.deps);
            }
            let count = args
                .get(2)
                .map_or_else(|| all.clone(), |value| value.deps.clone().union(pc));
            let mut bytes = Value::dependent(all.clone());
            bytes.put(Projection::Length, &Value::dependent(count));
            let mut output = Value::dependent(all);
            output.put(Projection::Tag, &Value::dependent(bounds.clone()));
            output.put(Projection::Variant("Ok".into(), 0), &bytes);
            output.put(
                Projection::Variant("Err".into(), 0),
                &Value::dependent(bounds),
            );
            return output;
        }
        if !external && matches!(name, "Bytes.concat" | "bytes_concat") {
            // Concatenation checks only the two extents and allocation-size
            // overflow before copying. It is fallible, but neither its status
            // nor successful extent depends on the copied byte values.
            let mut extent = pc.clone();
            for argument in args {
                extent.merge(&argument.project(Projection::Length).deps);
            }
            let mut bytes = Value::dependent(all.clone());
            bytes.put(Projection::Length, &Value::dependent(extent.clone()));
            let mut output = Value::dependent(all);
            output.put(Projection::Tag, &Value::dependent(extent.clone()));
            output.put(Projection::Variant("Ok".into(), 0), &bytes);
            output.put(
                Projection::Variant("Err".into(), 0),
                &Value::dependent(extent),
            );
            return output;
        }
        if !external && matches!(name, "Bytes.concat_many" | "bytes_concat_many") {
            // A pure bounded join, including its bare builtin alias. Keep all
            // item, list, limit and caller dependencies on every projection;
            // this does not infer a public extent or disclose a result tag.
            return Value::dependent(all);
        }
        if !external && matches!(name, "Bytes.write_u64_be" | "bytes_write_u64_be") {
            // A typed U64 always fits this exact eight-byte write. Its status
            // and encoded size cannot depend on integer bits. Keep those bits
            // on the Ok payload, and keep caller control on every projection.
            // Narrowing/text/variable-width writers do not have this proof.
            let public = Value::dependent(pc.clone());
            let mut encoded = Value::dependent(all.clone());
            encoded.put(Projection::Length, &public);
            let mut output = Value::dependent(all);
            output.put(Projection::Tag, &public);
            output.put(Projection::Variant("Ok".into(), 0), &encoded);
            output.put(Projection::Variant("Err".into(), 0), &public);
            return output;
        }
        if !external
            && matches!(
                name,
                "Result.is_ok" | "Result.is_err" | "Option.is_some" | "Option.is_none"
            )
        {
            let mut value = args
                .first()
                .cloned()
                .unwrap_or_default()
                .project(Projection::Tag);
            value.control(pc);
            return value;
        }
        if !external && matches!(name, "Result.unwrap_or" | "Option.unwrap_or") {
            let input = args.first().cloned().unwrap_or_default();
            let variant = if name.starts_with("Result.") {
                "Ok"
            } else {
                "Some"
            };
            let values = [
                input.project(Projection::Variant(variant.into(), 0)),
                args.get(1).cloned().unwrap_or_default(),
            ];
            return Value::alternatives(
                &values,
                &pc.clone().union(&input.project(Projection::Tag).deps),
            );
        }
        if !external
            && matches!(
                name,
                "List.length" | "list_length" | "Bytes.length" | "bytes_length"
            )
        {
            let mut value = args
                .first()
                .cloned()
                .unwrap_or_default()
                .project(Projection::Length);
            value.control(pc);
            return value;
        }
        if !external && matches!(name, "List.head" | "list_head" | "List.get" | "list_get") {
            let input = args.first().cloned().unwrap_or_default();
            let mut value = input.project(Projection::Item);
            let mut control = pc.clone().union(&input.project(Projection::Length).deps);
            for argument in args.iter().skip(1) {
                control.merge(&argument.deps);
            }
            value.control(&control);
            return value;
        }
        if !external
            && matches!(
                name,
                "List.reverse"
                    | "list_reverse"
                    | "List.drop"
                    | "list_drop"
                    | "List.take"
                    | "list_take"
                    | "List.tail"
                    | "list_tail"
            )
        {
            let mut value = args.first().cloned().unwrap_or_default();
            let mut control = pc.clone();
            for argument in args.iter().skip(1) {
                control.merge(&argument.deps);
            }
            value.control(&control);
            return value;
        }
        if !external
            && matches!(
                name,
                "List.append" | "list_append" | "List.concat" | "list_concat"
            )
        {
            let left = args.first().cloned().unwrap_or_default();
            let right = args.get(1).cloned().unwrap_or_default();
            let concat = matches!(name, "List.concat" | "list_concat");
            let mut length = left.project(Projection::Length);
            if concat {
                length.deps.merge(&right.project(Projection::Length).deps);
            }
            let items = [
                left.project(Projection::Item),
                if concat {
                    right.project(Projection::Item)
                } else {
                    right
                },
            ];
            let mut output = Value::dependent(pc.clone());
            output.put(Projection::Length, &length);
            output.put(Projection::Item, &Value::alternatives(&items, pc));
            return output;
        }
        if !external && matches!(name, "List.map" | "list_map") {
            let input = args.first().cloned().unwrap_or_default();
            let callback = args.get(1).cloned().unwrap_or_default();
            let control = pc.clone().union(&input.project(Projection::Length).deps);
            let item = input.project(Projection::Item);
            let output_item = if let Some(closure) = &callback.closure {
                self.invoke_closure(closure, &[item], &control)
            } else {
                self.call(
                    callback.callable.as_deref(),
                    &[],
                    &[item],
                    &control.union(&callback.deps),
                    span,
                )
            };
            let mut output = Value::dependent(pc.clone());
            output.put(Projection::Length, &input.project(Projection::Length));
            output.put(Projection::Item, &output_item);
            return output;
        }
        if !external && matches!(name, "BytesBuilder.new" | "bytes_builder_new") {
            let location = BuilderLocation::Local(u32::from(span.start()), u32::from(span.end()));
            // One allocation site can run repeatedly (loops or a called
            // closure). Its abstraction must never reset an older alias's
            // contents/escape fact when a later allocation executes.
            self.builders
                .entry(location.clone())
                .or_default()
                .deps
                .merge(&all);
            return Value {
                deps: all,
                builders: BTreeSet::from([location]),
                ..Value::default()
            };
        }
        if !external
            && matches!(
                name,
                "BytesBuilder.write_bytes"
                    | "BytesBuilder.write_u8"
                    | "BytesBuilder.write_u16_be"
                    | "BytesBuilder.write_u32_be"
                    | "bytes_builder_write_bytes"
                    | "bytes_builder_write_u8"
                    | "bytes_builder_write_u16_be"
                    | "bytes_builder_write_u32_be"
            )
        {
            self.write_builder(
                &args.first().cloned().unwrap_or_default(),
                all.clone(),
                span,
            );
            return Value::dependent(all);
        }
        if !external && matches!(name, "BytesBuilder.finish" | "bytes_builder_finish") {
            return Value::dependent(all);
        }
        // The genuine destructor zeroizes a local opaque handle. It exports no
        // material. Preserve the value/control dependency so using its result
        // cannot turn this local operation into a declassification primitive.
        if !external && matches!(name, "Secret.destroy" | "secret_destroy") {
            return Value::dependent(all);
        }
        if !external && matches!(name, "declassify" | "declassify_discriminant") {
            let reason = if syntax_args.len() == 2 {
                literal_reason(&syntax_args[1])
            } else {
                None
            };
            if let Some(reason) = reason {
                if self.emit {
                    self.sites.insert(
                        (u32::from(span.start()), u32::from(span.end()), name.into()),
                        PlaintextSite {
                            kind: name.into(),
                            reason,
                            span,
                        },
                    );
                }
                if name == "declassify_discriminant" {
                    let mut output = args.first().cloned().unwrap_or_default();
                    // This is an explicit override of exactly one projection.
                    // Preserve the whole value and payload dependencies, prior
                    // shape control, early exits and local resource/builder state.
                    // Current caller control cannot be reviewed through this API.
                    output.shape.insert(vec![Projection::Tag], pc.clone());
                    output.control(pc);
                    return output;
                }
                return Value {
                    declassified: true,
                    ..Value::default()
                };
            }
            self.error(span, name, "supply a nonempty, literal review reason");
            return Value::dependent(all);
        }
        let seal_data = if !external {
            match name {
                "Crypto.aead_seal"
                | "crypto_aead_seal"
                | "Crypto.xchacha20poly1305_seal"
                | "crypto_xchacha20poly1305_seal"
                | "Crypto.hpke_seal"
                | "crypto_hpke_seal" => Some(3),
                "StorageKey.seal_bytes" | "storage_key_seal_bytes" => Some(0),
                "Ohttp.encapsulate_request" | "ohttp_encapsulate_request" => Some(2),
                "Ohttp.encapsulate_response" | "ohttp_encapsulate_response" => Some(1),
                _ => None,
            }
        } else {
            None
        };
        if let Some(data) = seal_data {
            let mut public = pc.clone();
            for (index, value) in args.iter().enumerate() {
                if index != data {
                    public.merge(&value.deps);
                }
            }
            self.effect("seal public metadata", public.clone(), span);
            // Only the successful bytes crossed the encryption boundary.
            // Bounded seal APIs can fail because of the input length, and
            // InvalidLength carries that actual length in its error payload.
            // Keeping the length projection (rather than the private bits)
            // also preserves genuinely public-size framing proofs.
            let failure = args
                .get(data)
                .cloned()
                .unwrap_or_default()
                .project(Projection::Length)
                .deps
                .union(&public);
            let mut output = Value::dependent(failure.clone());
            output.put(Projection::Tag, &Value::dependent(failure.clone()));
            let mut successful = Value::dependent(public.clone());
            if matches!(
                name,
                "Ohttp.encapsulate_request" | "ohttp_encapsulate_request"
            ) {
                // Only the wire field is ciphertext. The opaque response context
                // is not an export/declassification boundary for the input.
                successful.put(Projection::Field("wire".into()), &Value::dependent(public));
                successful.put(
                    Projection::Field("response_context".into()),
                    &Value::dependent(all.clone()),
                );
            }
            output.put(Projection::Variant("Ok".into(), 0), &successful);
            output.put(
                Projection::Variant("Err".into(), 0),
                &Value::dependent(failure),
            );
            return output;
        }
        if !external
            && matches!(
                name,
                "Ohttp.decapsulate_request"
                    | "ohttp_decapsulate_request"
                    | "Ohttp.decapsulate_response"
                    | "ohttp_decapsulate_response"
            )
        {
            // Opening is pure, but neither its payload nor tag releases private
            // source/control dependencies. OHTTP is not a plaintext source label.
            return Value::dependent(all);
        }
        let pure_module = matches!(
            base,
            "String"
                | "Bytes"
                | "Int"
                | "Float"
                | "Bool"
                | "U64"
                | "U128"
                | "I128"
                | "List"
                | "Map"
                | "Set"
                | "Tuple"
                | "Option"
                | "Result"
                | "Queue"
                | "Iter"
                | "Math"
                | "Regex"
                | "Crypto"
        );
        if pure_module && !external {
            for (index, argument) in args.iter().enumerate() {
                if let Some(name) = &argument.callable {
                    self.summary_reads.insert(name.clone());
                }
                if let Some(closure) = &argument.closure {
                    let arity = closure
                        .syntax
                        .param_list()
                        .map_or(0, |list| list.params().count());
                    self.invoke_closure(closure, &vec![Value::dependent(all.clone()); arity], pc);
                } else if let Some(summary) = argument
                    .callable
                    .as_ref()
                    .and_then(|name| self.summaries.get(name))
                    .cloned()
                {
                    for (sink, deps) in summary.effects {
                        let arity = deps.parameters.last().map_or(0, |index| index + 1);
                        self.effect(
                            &sink,
                            deps.substitute(&vec![Value::dependent(all.clone()); arity])
                                .union(pc),
                            span,
                        );
                    }
                } else if syntax_args
                    .get(index)
                    .and_then(|expr| self.types.get(&expr.syntax().text_range()))
                    .is_some_and(|ty| matches!(ty, Ty::Fun(..)))
                {
                    self.effect("unknown higher-order callback", all.clone(), span);
                }
            }
        }
        let constructor =
            name.chars().next().is_some_and(char::is_uppercase) && !name.contains('.');
        if constructor && !external {
            let mut value = Value::dependent(all);
            value.variants = Some(BTreeSet::from([name.into()]));
            value.put(Projection::Tag, &Value::dependent(pc.clone()));
            for (index, argument) in args.iter().enumerate() {
                value.put(Projection::Variant(name.into(), index), argument);
                value.builders.extend(argument.builders.iter().cloned());
            }
            return value;
        }
        if external || !(pure_module || constructor) {
            for argument in args {
                self.escape_builder(argument);
            }
            self.effect(name, all.clone(), span);
        }
        Value {
            deps: all,
            builders: if constructor {
                args.iter()
                    .flat_map(|value| value.builders.iter().cloned())
                    .collect()
            } else {
                BTreeSet::new()
            },
            ..Value::default()
        }
    }

    fn invoke_closure(
        &mut self,
        closure: &ClosureContext,
        arguments: &[Value],
        pc: &Dependencies,
    ) -> Value {
        let mut scope = closure.captured.clone();
        for (index, param) in closure
            .syntax
            .param_list()
            .into_iter()
            .flat_map(|list| list.params().collect::<Vec<_>>())
            .enumerate()
        {
            let mut value = arguments.get(index).cloned().unwrap_or_default();
            if let Some(annotation) = param.type_annotation() {
                self.annotate(&mut value, &annotation);
            }
            if let Some(name) = param.name() {
                scope.insert(name.text().into(), value.clone());
            }
            if let Some(pattern) = param.pattern() {
                self.bind_pattern(pattern.syntax(), &value, &mut scope);
            }
        }
        let prior = std::mem::take(&mut self.current.output);
        let prior_returns = std::mem::take(&mut self.returned);
        let output = closure
            .syntax
            .body()
            .map(|body| self.block(&body, &mut scope, pc))
            .unwrap_or_default();
        self.returned.push(output);
        let mut output = Value::alternatives(&self.returned, pc);
        // A closure's `return`/`?` returns to this call, not from its caller.
        // Its result shape keeps the branch dependencies; only argument
        // evaluation can introduce a caller-side implicit early return.
        output.exits = Dependencies::default();
        self.current.output = prior;
        self.returned = prior_returns;
        output
    }
}

fn join(values: &[Value]) -> Dependencies {
    values
        .iter()
        .fold(Dependencies::default(), |mut deps, value| {
            deps.merge(&value.deps);
            deps
        })
}

fn pure_receiver_module(ty: &Ty) -> Option<&str> {
    match ty {
        Ty::Con(con)
            if matches!(
                con.name.as_str(),
                "String"
                    | "Bytes"
                    | "Int"
                    | "Float"
                    | "Bool"
                    | "U64"
                    | "U128"
                    | "I128"
                    | "List"
                    | "Map"
                    | "Set"
                    | "Queue"
                    | "Iter"
                    | "Regex"
            ) =>
        {
            Some(&con.name)
        }
        Ty::App(base, _) => pure_receiver_module(base),
        _ => None,
    }
}

fn contains_builder(ty: &Ty) -> bool {
    match ty {
        Ty::Fun(..) => false,
        Ty::Con(con) => con.name == "BytesBuilder",
        _ => ty.parts().any(contains_builder),
    }
}

fn has_control_exit(expression: &Expr) -> bool {
    expression.syntax().descendants().any(|node| {
        matches!(
            node.kind(),
            SyntaxKind::RETURN_EXPR
                | SyntaxKind::TRY_EXPR
                | SyntaxKind::BREAK_EXPR
                | SyntaxKind::CONTINUE_EXPR
        )
    })
}

fn literal_reason(expr: &Expr) -> Option<String> {
    let Expr::StringExpr(value) = expr else {
        return None;
    };
    if value
        .syntax()
        .descendants()
        .any(|node| node.kind() == SyntaxKind::INTERPOLATION)
    {
        return None;
    }
    let text = value
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::STRING_CONTENT)
        .map(|token| token.text().to_string())
        .collect::<String>();
    review_reason(&text)
}

fn review_reason(raw: &str) -> Option<String> {
    let mut text = String::new();
    let mut chars = raw.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }
        match chars.next()? {
            'n' => text.push('\n'),
            'r' => text.push('\r'),
            't' => text.push('\t'),
            '0' => text.push('\0'),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut digits = String::new();
                loop {
                    let next = chars.next()?;
                    if next == '}' {
                        break;
                    }
                    digits.push(next);
                }
                text.push(char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?);
            }
            other => text.push(other),
        }
    }
    (!text.trim().is_empty() && !text.chars().any(char::is_control)).then_some(text)
}

#[cfg(test)]
mod dependency_tests {
    use super::{Dependencies, Projection};

    #[test]
    fn dependency_union_preserves_exact_data_and_control_coverage() {
        let paths = [
            vec![Projection::Tag],
            vec![Projection::PriorControl],
            vec![Projection::Field("a".into())],
            vec![Projection::Field("a".into()), Projection::Tag],
            vec![Projection::Field("a".into()), Projection::PriorControl],
            vec![Projection::Field("b".into()), Projection::Tag],
        ];
        let facts = |mask: usize| {
            let mut value = Dependencies::default();
            if mask & 1 != 0 {
                value.parameters.insert(0);
            }
            for (index, path) in paths[..4].iter().enumerate() {
                if mask & (2 << index) != 0 {
                    value.projections.insert((index % 2, path.clone()));
                }
            }
            value.intrinsic = mask & 32 != 0;
            value
        };
        let covers = |value: &Dependencies, index, path: &[Projection]| {
            value.parameters.contains(&index)
                || value
                    .projections
                    .iter()
                    .any(|(parameter, prefix)| *parameter == index && path.starts_with(prefix))
        };
        for left in 0..64 {
            for right in 0..64 {
                let first = facts(left);
                let second = facts(right);
                let merged = first.clone().union(&second);
                assert_eq!(merged.intrinsic, first.intrinsic || second.intrinsic);
                for index in 0..3 {
                    for path in &paths {
                        assert_eq!(
                            covers(&merged, index, path),
                            covers(&first, index, path) || covers(&second, index, path),
                            "{left}/{right} at {index}/{path:?}"
                        );
                    }
                }
            }
        }
    }
}
