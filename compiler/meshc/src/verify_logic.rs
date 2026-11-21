//! Binds Mesh logic files to checked program declarations and their proofs.

use mesh_diagnostics::{Code, Diagnostic, SourceFile};
use std::{collections::HashSet, fmt::Write as _};

#[path = "verify_logic_render.rs"]
mod render;

use mesh_codegen::mir::lower::LoweredSignature;
use mesh_parser::logic::{self, Arm, Binder, Declaration, Pattern, Proof, Term};
use mesh_typeck::ty::{NatIndex, TyCon, TyVar};
use mesh_typeck::{TypeRegistry, VariantFieldInfo};

use super::*;

#[derive(Default)]
pub(super) struct Program {
    imports: BTreeMap<String, Import>,
    specifications: BTreeMap<String, (Declaration, Location)>,
    proofs: BTreeMap<String, (Declaration, Location)>,
}

struct Import {
    module: String,
    module_id: ModuleId,
    source_name: String,
    binding: ImportBinding,
    requirements: String,
    location: Location,
}

#[derive(PartialEq, Eq)]
enum ImportBinding {
    Function {
        signature: Ty,
        request_specialization: bool,
    },
    Nominal {
        ty: Ty,
        constructors: Vec<String>,
    },
    Parametric {
        signature: Ty,
        parameters: Vec<String>,
    },
    NominalFamily {
        constructors: Vec<String>,
    },
}

pub(super) struct Theorem {
    pub(super) entry: InventoryEntry,
    pub(super) proof_location: Option<Location>,
}

pub(super) struct ProofFailure {
    pub(super) code: Code,
    pub(super) reason: String,
    pub(super) location: Option<Location>,
}

impl From<String> for ProofFailure {
    fn from(reason: String) -> Self {
        Self {
            code: Code::LawUnsupported,
            reason,
            location: None,
        }
    }
}

impl From<verify_solver::EncodingError> for ProofFailure {
    fn from(error: verify_solver::EncodingError) -> Self {
        Self {
            code: error.code,
            reason: error.reason,
            location: None,
        }
    }
}

impl Program {
    pub(super) fn add_file(
        &mut self,
        project: &ProjectData,
        checked: &CheckedProject,
        path: &str,
        source: &SourceFile,
        parsed: logic::LogicFile,
    ) -> Result<(), Diagnostic> {
        let lines = LineIndex::new(source.text().expect("logic source text"));
        self.add_parsed(project, checked, parsed, source, |span| {
            Location::new(path, &lines, span)
        })
    }

    pub(super) fn add_project_blocks(
        &mut self,
        project: &ProjectData,
        checked: &CheckedProject,
        files: &[SourceFile],
    ) -> Result<(), Diagnostic> {
        for &id in &project.compilation_order {
            let index = id.0 as usize;
            let tree = project.module_parses[index].tree();
            let mut blocks = tree.logic_blocks().peekable();
            if blocks.peek().is_none() {
                continue;
            }
            let path = project.graph.get(id).path.to_string_lossy();
            let source = &project.module_sources[index];
            let lines = LineIndex::new(source);
            for block in blocks {
                let range = block.body_range().ok_or_else(|| {
                    Diagnostic::internal(format!("Invalid logic block in {path}"))
                })?;
                let start = u32::from(range.start());
                let end = u32::from(range.end());
                let body = &source[start as usize..end as usize];
                let at = |span: Span| {
                    Location::new(
                        &path,
                        &lines,
                        Span::new(start + span.start, start + span.end),
                    )
                };
                // The frontend reports a syntax error in the block first.
                let parsed = logic::parse_body(body).map_err(|error| {
                    Diagnostic::internal(format!("{}: {}", at(error.span), error.message))
                })?;
                self.add_parsed(project, checked, parsed, &files[index], at)?;
            }
        }
        Ok(())
    }

    fn add_parsed(
        &mut self,
        project: &ProjectData,
        checked: &CheckedProject,
        parsed: logic::LogicFile,
        source: &SourceFile,
        at: impl Fn(Span) -> Location,
    ) -> Result<(), Diagnostic> {
        for import in parsed.imports {
            let candidate = bind_import(project, checked, &import, at(import.span), source)?;
            let name = import.alias.unwrap_or(import.name);
            if let Some(previous) = self.imports.get(&name) {
                if previous.module_id != candidate.module_id
                    || previous.source_name != candidate.source_name
                    || previous.binding != candidate.binding
                {
                    return Err(Diagnostic::in_file(
                        Code::LawBindingUnresolved,
                        source,
                        format!(
                            "Logic import `{name}` is ambiguous with the import at {}",
                            previous.location
                        ),
                    )
                    .at(candidate.location.span(), ""));
                }
            } else {
                self.imports.insert(name, candidate);
            }
        }
        for declaration in parsed.declarations {
            let (name, span) = name_and_span(&declaration);
            let name = name.to_owned();
            let location = at(span);
            let collection = if matches!(declaration, Declaration::Proof { .. }) {
                &mut self.proofs
            } else {
                &mut self.specifications
            };
            if let Some((_, previous)) = collection.get(&name) {
                return Err(Diagnostic::in_file(
                    Code::LawBindingUnresolved,
                    source,
                    format!("Duplicate logic declaration `{name}` at {previous}"),
                )
                .at(location.span(), ""));
            }
            collection.insert(name, (declaration, location));
        }
        Ok(())
    }

    pub(super) fn inventory(&self, package: &str) -> Result<Vec<Theorem>, String> {
        let scope = self.dependency_scope();
        if self.specifications.values().any(|(declaration, _)| {
            matches!(declaration, Declaration::Theorem { public: true, .. })
        }) {
            for sort in ["Nat", "Prop"] {
                if scope.globals.contains(sort) || scope.constructors.contains_key(sort) {
                    return Err(format!(
                        "Logic name `{sort}` shadows a builtin sort used by exported proof values"
                    ));
                }
            }
        }
        for (name, (_, location)) in &self.proofs {
            if !matches!(
                self.specifications.get(name),
                Some((Declaration::Theorem { .. }, _))
            ) {
                return Err(format!(
                    "{location}: Proof `{name}` has no corresponding theorem"
                ));
            }
        }
        let mut theorems = Vec::new();
        for (name, (declaration, location)) in &self.specifications {
            let Declaration::Theorem { params, .. } = declaration else {
                continue;
            };
            theorems.push(Theorem {
                entry: InventoryEntry {
                    target: Target {
                        package: package.to_owned(),
                        module: "<logic>".to_owned(),
                        function: name.clone(),
                        arity: params.len(),
                        signature: "Prop".to_owned(),
                    },
                    access: Access::Theorem,
                    requirement_digest: self.requirement_digest(name, &scope),
                    location: location.clone(),
                    declaration: location.clone(),
                },
                proof_location: self.proofs.get(name).map(|(_, location)| location.clone()),
            });
        }
        if theorems.is_empty() && (!self.specifications.is_empty() || !self.imports.is_empty()) {
            return Err("Logic declarations need at least one theorem".into());
        }
        Ok(theorems)
    }

    pub(super) fn verify(
        &self,
        project: &ProjectData,
        checked: &CheckedProject,
        modules: &[MirModule],
        signatures: &[Vec<LoweredSignature>],
        settings: verify_proof::Settings<'_>,
    ) -> Result<(), ProofFailure> {
        let declarations = verify_proof::nominal_declarations(project, checked);
        let callees = verify_proof::checked_callees(project, checked, modules, signatures);
        let mut model =
            verify_proof::ModelSession::new(modules, &declarations).with_callees(callees);
        let mut compiled = render::NativeImports::default();
        let mut aliases = String::new();
        let mut templates = String::new();
        for (name, import) in &self.imports {
            let registry = &checked.typeck[import.module_id.0 as usize].type_registry;
            let at_import = |error: verify_solver::EncodingError| ProofFailure {
                code: error.code,
                reason: format!("{}: {}", import.location, error.reason),
                location: error
                    .location
                    .and_then(|span| project_expression_location(project, span)),
            };
            match &import.binding {
                ImportBinding::Function { signature, .. } => {
                    let idx = project
                        .compilation_order
                        .iter()
                        .position(|id| *id == import.module_id)
                        .ok_or_else(|| {
                            format!("{}: No lowered module `{}`", import.location, import.module)
                        })?;
                    let entries = signatures[idx]
                        .iter()
                        .filter(|entry| {
                            entry.source_name == import.source_name && &entry.signature == signature
                        })
                        .collect::<Vec<_>>();
                    let [entry] = entries.as_slice() else {
                        return Err(format!(
                            "{}: Expected one checked body for `{}.{}` at {signature}, found {}",
                            import.location,
                            import.module,
                            import.source_name,
                            entries.len()
                        )
                        .into());
                    };
                    let function = lowered_function(&modules[idx], &entry.symbol)?;
                    verify_proof::require_unambiguous_definition(modules, function)
                        .map_err(at_import)?;
                    let emitted = model.import(&entry.symbol).map_err(at_import)?;
                    compiled.names.insert(name.clone(), emitted);
                }
                ImportBinding::Nominal { ty, .. } => {
                    let alias = format!("mesh_imported_type_{}", compiled.names.len());
                    let ty_expression = model.source_type(ty, registry).map_err(at_import)?;
                    writeln!(aliases, "abbrev {alias} := {ty_expression}").unwrap();
                    for (variant, emitted, fields) in model
                        .nominal_constructors(ty, registry)
                        .map_err(at_import)?
                    {
                        let term =
                            format!("mesh_imported_constructor_{}", compiled.constructors.len());
                        write!(aliases, "abbrev {term}").unwrap();
                        for (index, field) in fields.iter().enumerate() {
                            write!(aliases, " (field{index} : {field})").unwrap();
                        }
                        write!(aliases, " : {alias} := {emitted}").unwrap();
                        for index in 0..fields.len() {
                            write!(aliases, " field{index}").unwrap();
                        }
                        aliases.push('\n');
                        compiled.constructors.insert(
                            (name.clone(), variant),
                            render::NativeConstructor {
                                symbol: emitted,
                                term_alias: Some(term),
                            },
                        );
                    }
                    compiled.names.insert(name.clone(), alias);
                }
                ImportBinding::NominalFamily { .. } => {
                    let (family, constructors) = model
                        .nominal_family(&import.source_name, registry)
                        .map_err(at_import)?;
                    compiled.names.insert(name.clone(), family);
                    for (variant, symbol) in constructors {
                        compiled.constructors.insert(
                            (name.clone(), variant),
                            render::NativeConstructor {
                                symbol,
                                term_alias: None,
                            },
                        );
                    }
                }
                ImportBinding::Parametric {
                    signature,
                    parameters,
                } => {
                    let idx = project
                        .compilation_order
                        .iter()
                        .position(|id| *id == import.module_id)
                        .ok_or_else(|| {
                            format!("{}: No lowered module `{}`", import.location, import.module)
                        })?;
                    let (template, entries) = lower_template(project, checked, import, signature)?;
                    let mut roots = entries.iter().filter(|entry| &entry.signature == signature);
                    let (Some(entry), None) = (roots.next(), roots.next()) else {
                        return Err(format!(
                            "{}: Expected one checked template for `{}.{}`",
                            import.location, import.module, import.source_name
                        )
                        .into());
                    };
                    let function = lowered_function(&template, &entry.symbol)?;
                    let mut rigid = Vec::new();
                    for candidate in &entries {
                        let definition = lowered_function(&template, &candidate.symbol)?;
                        verify_proof::require_unambiguous_definition(
                            std::slice::from_ref(&template),
                            definition,
                        )
                        .map_err(at_import)?;
                        rigid.push((definition, candidate));
                    }
                    verify_proof::require_unshadowed_runtime(modules).map_err(at_import)?;
                    let mut native = Vec::new();
                    let indexed = parameters
                        .iter()
                        .any(|name| name.starts_with("\0mesh_index_"));
                    for actual in signatures[idx].iter().filter(|actual| {
                        actual.source_name == import.source_name
                            && !crate::has_runtime_type_variables(&actual.signature)
                    }) {
                        let concrete = lowered_function(&modules[idx], &actual.symbol)?;
                        verify_proof::require_unambiguous_definition(modules, concrete)
                            .map_err(at_import)?;
                        if indexed {
                            model
                                .validate_indexed_native_layouts(actual, registry)
                                .map_err(at_import)?;
                        } else {
                            model.import(&actual.symbol).map_err(at_import)?;
                        }
                        native.push((concrete, actual));
                    }
                    let aliases = crate::verify_parametric::check_specializations(
                        (function, entry),
                        &rigid,
                        &native,
                        parameters,
                        registry,
                    )
                    .map_err(|reason| {
                        format!(
                        "{}: Specializations of `{}.{}` differ from its generic template: {reason}",
                        import.location, import.module, import.source_name
                    )
                    })?;
                    let symbol = format!("mesh_generic_{}", compiled.names.len());
                    templates.push_str(
                        &model
                            .parametric_definition_with_recursive_aliases(
                                function, entry, registry, parameters, &symbol, &aliases,
                            )
                            .map_err(at_import)?,
                    );
                    compiled.names.insert(name.clone(), symbol);
                }
            }
        }
        let declarations = self.ordered_declarations()?;
        let rendered = render::render(&declarations, &compiled)?;
        let mut source = verify_proof::PRELUDE.to_owned();
        source.push_str(model.prelude());
        source.push_str(&aliases);
        source.push_str(model.helper_definitions());
        source.push_str(&templates);
        source.push_str(&rendered.source);
        verify_proof::check_artifact(source, &rendered.theorem_symbols, settings)
            .map_err(Into::into)
    }

    pub(super) fn verification_lowering(&self) -> crate::VerificationLowering {
        let mut lowering = crate::VerificationLowering::default();
        for import in self.imports.values() {
            if let ImportBinding::Function {
                signature,
                request_specialization: true,
            } = &import.binding
            {
                crate::push_usage_type(&mut lowering.requested, &import.source_name, signature);
            }
        }
        lowering
    }

    fn dependency_scope(&self) -> DependencyScope<'_> {
        let globals = self
            .specifications
            .keys()
            .chain(self.imports.keys())
            .map(String::as_str)
            .collect();
        let mut constructors: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (name, (declaration, _)) in &self.specifications {
            if let Declaration::Inductive {
                constructors: variants,
                ..
            } = declaration
            {
                for variant in variants {
                    constructors.entry(&variant.name).or_default().insert(name);
                }
            }
        }
        for (name, import) in &self.imports {
            if let ImportBinding::Nominal {
                constructors: variants,
                ..
            }
            | ImportBinding::NominalFamily {
                constructors: variants,
            } = &import.binding
            {
                for variant in variants {
                    constructors.entry(variant).or_default().insert(name);
                }
            }
        }
        DependencyScope {
            globals,
            constructors,
        }
    }

    fn requirement_digest(&self, name: &str, scope: &DependencyScope<'_>) -> String {
        // A statement includes every specification definition and concrete
        // import it depends on. Proof bodies and source locations are excluded.
        let mut dependencies = BTreeSet::new();
        let mut pending = vec![name.to_owned()];
        while let Some(name) = pending.pop() {
            if dependencies.insert(name.clone()) {
                if let Some((declaration, _)) = self.specifications.get(&name) {
                    pending.extend(references(declaration, scope, &[]));
                }
            }
        }
        let mut hash = Sha256::new();
        hash.update(b"mesh-logic-specification-v1\0");
        for name in dependencies {
            if let Some((declaration, _)) = self.specifications.get(&name) {
                hash.update(specification_text(declaration));
                hash.update([0]);
            }
            if let Some(import) = self.imports.get(&name) {
                hash.update(format!(
                    "import {}.{} {}",
                    import.module, import.source_name, import.requirements
                ));
                hash.update([0]);
            }
        }
        format!("{:x}", hash.finalize())
    }

    fn ordered_declarations(&self) -> Result<Vec<Declaration>, String> {
        let scope = self.dependency_scope();
        let dependencies: BTreeMap<_, _> = self
            .specifications
            .iter()
            .map(|(name, (declaration, _))| {
                let mut needs = references(declaration, &scope, &[]);
                if let (Some((proof, _)), Declaration::Theorem { params, .. }) =
                    (self.proofs.get(name), declaration)
                {
                    needs.extend(references(proof, &scope, params));
                }
                needs.retain(|needed| self.specifications.contains_key(needed));
                // Recursive definition bodies are checked for totality by Lean.
                if matches!(
                    declaration,
                    Declaration::Definition { .. } | Declaration::Inductive { .. }
                ) {
                    needs.remove(name);
                }
                (name, needs)
            })
            .collect();
        let mut ordered = Vec::new();
        let mut done = BTreeSet::new();
        while done.len() < dependencies.len() {
            let Some((name, _)) = dependencies.iter().find(|(name, needs)| {
                !done.contains(name.as_str())
                    && needs.iter().all(|dependency| done.contains(dependency))
            }) else {
                let stuck = dependencies
                    .keys()
                    .find(|name| !done.contains(name.as_str()))
                    .unwrap();
                return Err(format!(
                    "{}: Cyclic logic dependency involving `{stuck}`",
                    self.specifications[*stuck].1
                ));
            };
            done.insert((*name).clone());
            ordered.push(self.specifications[*name].0.clone());
            if let Some((proof, _)) = self.proofs.get(*name) {
                ordered.push(proof.clone());
            }
        }
        Ok(ordered)
    }
}

fn bind_import(
    project: &ProjectData,
    checked: &CheckedProject,
    import: &logic::LogicImport,
    location: Location,
    source: &SourceFile,
) -> Result<Import, Diagnostic> {
    let module_name = &import.module;
    let name = import.name.as_str();
    let reference = format!("{module_name}.{name}");
    let at = |code, reason: &str| {
        Diagnostic::in_file(code, source, format!("Logic import `{reference}` {reason}"))
            .at(location.span(), "")
    };
    let invalid = |reason| at(Code::LawBindingUnresolved, reason);
    let signature_mismatch = |reason| at(Code::LawSignatureMismatch, reason);
    let module_id = project
        .graph
        .resolve(module_name)
        .ok_or_else(|| invalid("has no module"))?;
    let idx = module_id.0 as usize;
    let module = project.graph.get(module_id);
    if module.path.is_absolute() {
        return Err(invalid("must belong to this package"));
    }
    let exports = checked.exports[idx]
        .as_ref()
        .ok_or_else(|| invalid("has no checked module"))?;
    if checked.typeck[idx].overloaded_fn_names.contains(name) {
        return Err(invalid("cannot select an overloaded function"));
    }
    let (binding, requirements) = if let Some(function) = exports.functions.get(name) {
        if function.ty.is_proof() || function.ty.is_callable_theorem() {
            return Err(invalid(
                "selects proof evidence without a native function body",
            ));
        }
        if !import.arguments.is_empty() {
            return Err(invalid("selects a function with nominal type arguments"));
        }
        let registry = &checked.typeck[idx].type_registry;
        let specialized = function.ty.has_type_vars();
        let signature = match &import.signature {
            Some((params, result)) => {
                let parse_type = |term| {
                    source_type(term, registry).ok_or_else(|| {
                        signature_mismatch("has an unsupported type in its signature")
                    })
                };
                let chosen = Ty::Fun(
                    params.iter().map(parse_type).collect::<Result<_, _>>()?,
                    Box::new(parse_type(result)?),
                );
                if !signature_instance(&function.ty, &chosen, &mut BTreeMap::new()) {
                    return Err(signature_mismatch(
                        "has a signature that is not an instance of its checked declaration",
                    ));
                }
                chosen
            }
            None if specialized => {
                if exports.function_constraints.contains_key(name) {
                    return Err(signature_mismatch(
                        "needs an explicit concrete signature for its constrained type parameters",
                    ));
                }
                let (parameters, signature) = rigid_signature(&function.ty, &function.vars)
                    .ok_or_else(|| {
                        signature_mismatch(
                            "needs an explicit concrete signature for its type parameters",
                        )
                    })?;
                return Ok(Import {
                    module: module.name.clone(),
                    module_id,
                    source_name: name.to_owned(),
                    binding: ImportBinding::Parametric {
                        signature,
                        parameters,
                    },
                    requirements: declaration_requirements(&function.ty, registry),
                    location,
                });
            }
            None => function.ty.clone(),
        };
        let requirements = declaration_requirements(&signature, registry);
        // A concrete signature alone cannot satisfy a trait constraint.
        // Constrained functions must already have a matching checked use.
        let request_specialization =
            specialized && !exports.function_constraints.contains_key(name);
        (
            ImportBinding::Function {
                signature,
                request_specialization,
            },
            requirements,
        )
    } else {
        if import.signature.is_some() {
            return Err(invalid("has no public function for the supplied signature"));
        }
        let (arity, constructors) = if let Some(sum) = exports.sum_type_defs.get(name) {
            (
                sum.generic_params.len(),
                sum.variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect(),
            )
        } else if let Some(record) = exports.struct_defs.get(name) {
            (record.generic_params.len(), vec!["mk".to_owned()])
        } else {
            return Err(invalid("has no public function or inductive type"));
        };
        if arity > 0 && import.arguments.is_empty() {
            let registry = &checked.typeck[idx].type_registry;
            return Ok(Import {
                module: module.name.clone(),
                module_id,
                source_name: name.to_owned(),
                binding: ImportBinding::NominalFamily { constructors },
                requirements: declaration_requirements(
                    &nominal_type(name, Vec::new(), registry),
                    registry,
                ),
                location,
            });
        }
        if import.arguments.len() != arity {
            return Err(at(
                Code::LawSignatureMismatch,
                &format!("needs {arity} concrete type arguments"),
            ));
        }
        let registry = &checked.typeck[idx].type_registry;
        let args = nominal_arguments(name, &import.arguments, registry)
            .ok_or_else(|| invalid("has an unsupported type argument"))?;
        let ty = nominal_type(name, args, registry);
        let requirements = declaration_requirements(&ty, registry);
        (ImportBinding::Nominal { ty, constructors }, requirements)
    };
    Ok(Import {
        module: module.name.clone(),
        module_id,
        source_name: name.to_owned(),
        binding,
        requirements,
        location,
    })
}

fn rigid_signature(signature: &Ty, vars: &[TyVar]) -> Option<(Vec<String>, Ty)> {
    fn nat_variables(ty: &Ty, found: &mut HashSet<TyVar>) {
        if let Ty::Nat(index) = ty {
            let mut inner = index;
            while let NatIndex::Succ(predecessor) = inner {
                inner = predecessor;
            }
            if let NatIndex::Variable(var) = inner {
                found.insert(*var);
            }
        }
        for part in ty.parts() {
            nat_variables(part, found);
        }
    }
    let mut indices = HashSet::new();
    nat_variables(signature, &mut indices);
    let parameters: Vec<_> = vars
        .iter()
        .enumerate()
        .map(|(index, var)| {
            let kind = if indices.contains(var) {
                "index"
            } else {
                "type"
            };
            format!("\0mesh_{kind}_{index}")
        })
        .collect();
    fn rigid(ty: &Ty, variables: &[TyVar], parameters: &[String]) -> Ty {
        let name = |var: &TyVar| {
            variables
                .iter()
                .position(|candidate| candidate == var)
                .map(|index| &parameters[index])
        };
        match ty {
            Ty::Var(var) => name(var)
                .map(|name| TyCon::new(name))
                .map(Ty::Con)
                .unwrap_or_else(|| ty.clone()),
            Ty::Nat(index) => Ty::Nat(index.map_leaf(|leaf| {
                match leaf {
                    NatIndex::Variable(var) => name(var)
                        .map(|name| NatIndex::Parameter(name.clone()))
                        .unwrap_or_else(|| leaf.clone()),
                    _ => leaf.clone(),
                }
            })),
            _ => ty.map_parts(|part| rigid(part, variables, parameters)),
        }
    }
    let signature = rigid(signature, vars, &parameters);
    (!parameters.is_empty() && !signature.has_type_vars()).then_some((parameters, signature))
}

fn lower_template(
    project: &ProjectData,
    checked: &CheckedProject,
    import: &Import,
    signature: &Ty,
) -> Result<(MirModule, Vec<LoweredSignature>), String> {
    let index = import.module_id.0 as usize;
    let parse = &project.module_parses[index];
    let other_modules: Vec<_> = project
        .module_parses
        .iter()
        .zip(&checked.typeck)
        .enumerate()
        .filter_map(|(other, pair)| (other != index).then_some(pair))
        .collect();
    let public_functions = checked.exports[index]
        .as_ref()
        .map(|exports| exports.functions.keys().cloned().collect())
        .unwrap_or_default();
    let sources = source_ids(project);
    let (module, entries) = mesh_codegen::mir::lower::lower_module_to_mir_with_signatures(
        parse,
        &checked.typeck[index],
        &import.module,
        &public_functions,
        &[(import.source_name.clone(), vec![signature.clone()])].into(),
        &other_modules,
        |source| sources.get(&std::ptr::from_ref(source)).copied(),
    )
    .map_err(|error| {
        format!(
            "{}: Could not lower the generic template: {}",
            import.location,
            crate::diagnostic_reason(&error)
        )
    })?;
    let entries = entries
        .into_iter()
        .filter(|entry| entry.source_name == import.source_name)
        .collect();
    Ok((module, entries))
}

fn nominal_type(name: &str, args: Vec<Ty>, registry: &TypeRegistry) -> Ty {
    let head = Ty::Con(
        registry
            .sum_type_defs
            .get(name)
            .map_or_else(|| TyCon::new(name), mesh_typeck::SumTypeDefInfo::head),
    );
    if args.is_empty() {
        head
    } else {
        Ty::App(Box::new(head), args)
    }
}

fn nominal_arity(registry: &TypeRegistry, name: &str) -> Option<usize> {
    registry
        .struct_defs
        .get(name)
        .map(|record| record.generic_params.len())
        .or_else(|| {
            registry
                .sum_type_defs
                .get(name)
                .map(|sum| sum.generic_params.len())
        })
}

fn nominal_arguments(name: &str, args: &[Term], registry: &TypeRegistry) -> Option<Vec<Ty>> {
    let indices = registry
        .sum_type_defs
        .get(name)
        .and_then(|sum| sum.indexed.as_ref());
    args.iter()
        .enumerate()
        .map(|(position, arg)| {
            if indices.is_some_and(|schema| schema.index_positions.contains(&position)) {
                nat_index(arg).map(Ty::Nat)
            } else {
                source_type(arg, registry)
            }
        })
        .collect()
}

fn nat_index(term: &Term) -> Option<NatIndex> {
    match term {
        Term::Nat(value) => Some(NatIndex::Literal(*value)),
        Term::Call(head, args) => {
            let (Term::Name(path), [arg]) = (head.as_ref(), args.as_slice()) else {
                return None;
            };
            if path.as_slice() != ["Nat", "succ"] && path.as_slice() != ["Succ"] {
                return None;
            }
            match nat_index(arg)? {
                NatIndex::Literal(value) if value < u64::MAX => Some(NatIndex::Literal(value + 1)),
                index => Some(NatIndex::Succ(Box::new(index))),
            }
        }
        _ => None,
    }
}

fn source_type(term: &Term, registry: &TypeRegistry) -> Option<Ty> {
    match term {
        Term::Name(parts) => {
            let [name] = parts.as_slice() else {
                return None;
            };
            match name.as_str() {
                "Int" | "Bool" => Some(Ty::Con(TyCon::new(name))),
                "Unit" => Some(Ty::Tuple(Vec::new())),
                _ if nominal_arity(registry, name) == Some(0) => {
                    Some(nominal_type(name, Vec::new(), registry))
                }
                _ => None,
            }
        }
        Term::Call(head, items) => {
            let Term::Name(parts) = head.as_ref() else {
                return None;
            };
            let [name] = parts.as_slice() else {
                return None;
            };
            let arity = match name.as_str() {
                "List" => 1,
                "Tuple" if items.len() >= 2 => items.len(),
                _ => nominal_arity(registry, name)?,
            };
            if items.len() != arity {
                return None;
            }
            let arguments = nominal_arguments(name, items, registry)?;
            Some(if name == "Tuple" {
                Ty::Tuple(arguments)
            } else {
                nominal_type(name, arguments, registry)
            })
        }
        _ => None,
    }
}

fn signature_instance(signature: &Ty, chosen: &Ty, bindings: &mut BTreeMap<u32, Ty>) -> bool {
    match (signature, chosen) {
        (Ty::Var(TyVar(id)), _) => {
            return bindings.entry(*id).or_insert_with(|| chosen.clone()) == chosen
        }
        (Ty::Nat(NatIndex::Variable(TyVar(id))), Ty::Nat(_)) => {
            return bindings.entry(*id).or_insert_with(|| chosen.clone()) == chosen;
        }
        (Ty::Nat(NatIndex::Succ(inner)), Ty::Nat(index)) => {
            return index.predecessor().is_some_and(|predecessor| {
                signature_instance(
                    &Ty::Nat(inner.as_ref().clone()),
                    &Ty::Nat(predecessor),
                    bindings,
                )
            });
        }
        (Ty::Fun(params, _), Ty::Fun(other, _)) | (Ty::App(_, params), Ty::App(_, other))
            if params.len() == other.len() => {}
        (Ty::Tuple(fields), Ty::Tuple(other)) if fields.len() == other.len() => {}
        _ => return signature == chosen,
    }
    signature
        .parts()
        .zip(chosen.parts())
        .all(|(needed, supplied)| signature_instance(needed, supplied, bindings))
}

fn declaration_requirements(ty: &Ty, registry: &TypeRegistry) -> String {
    let mut definitions = BTreeMap::new();
    let mut visited = BTreeSet::new();
    let mut pending = vec![ty];
    while let Some(ty) = pending.pop() {
        pending.extend(ty.parts());
        let Ty::Con(con) = ty else { continue };
        if !visited.insert(&con.name) {
            continue;
        }
        if let Some(record) = registry.struct_defs.get(&con.name) {
            definitions.insert(
                &con.name,
                format!("{:?} {:?}", record.generic_params, record.fields),
            );
            pending.extend(record.fields.iter().map(|(_, field)| field));
        }
        if let Some(sum) = registry.sum_type_defs.get(&con.name) {
            definitions.insert(
                &con.name,
                format!("{:?} {:?}", sum.generic_params, sum.variants),
            );
            pending.extend(
                sum.variants
                    .iter()
                    .flat_map(|variant| &variant.fields)
                    .map(|field| match field {
                        VariantFieldInfo::Named(_, ty) | VariantFieldInfo::Positional(ty) => ty,
                    }),
            );
        }
    }
    let mut result = ty.to_string();
    for (name, definition) in definitions {
        writeln!(result, "\n{name} {definition}").unwrap();
    }
    result
}

fn name_and_span(declaration: &Declaration) -> (&str, Span) {
    match declaration {
        Declaration::Inductive { name, span, .. }
        | Declaration::Definition { name, span, .. }
        | Declaration::Theorem { name, span, .. }
        | Declaration::Proof { name, span, .. } => (name, *span),
    }
}

fn specification_text(declaration: &Declaration) -> String {
    match declaration {
        Declaration::Inductive {
            name,
            params,
            kind,
            constructors,
            ..
        } => {
            format!("type {name} {params:?} {kind:?} {constructors:?}")
        }
        Declaration::Definition {
            name,
            params,
            result,
            body,
            ..
        } => {
            format!("definition {name} {params:?} {result:?} {body:?}")
        }
        Declaration::Theorem {
            name,
            params,
            proposition,
            public,
            ..
        } => {
            let visibility = if *public { "pub " } else { "" };
            format!("{visibility}theorem {name} {params:?} {proposition:?}")
        }
        Declaration::Proof { .. } => unreachable!("proofs are not specifications"),
    }
}

fn references<'a>(
    declaration: &'a Declaration,
    scope: &DependencyScope<'a>,
    proof_params: &'a [Binder],
) -> BTreeSet<String> {
    let mut refs = References {
        scope,
        names: BTreeSet::new(),
        bindings: proof_params
            .iter()
            .map(|param| param.name.as_str())
            .collect(),
    };
    match declaration {
        Declaration::Inductive {
            params,
            kind,
            constructors,
            ..
        } => {
            refs.binders(params);
            if let Some(kind) = kind {
                refs.term(kind);
            }
            let saved = refs.bindings.len();
            for constructor in constructors {
                refs.binders(&constructor.fields);
                if let Some(result) = &constructor.result {
                    refs.term(result);
                }
                refs.bindings.truncate(saved);
            }
        }
        Declaration::Definition {
            params,
            result,
            body,
            ..
        } => {
            refs.binders(params);
            refs.term(result);
            refs.term(body);
        }
        Declaration::Theorem {
            params,
            proposition,
            ..
        } => {
            refs.binders(params);
            refs.term(proposition);
        }
        Declaration::Proof { term, .. } => refs.proof(term),
    }
    refs.names
}

struct DependencyScope<'a> {
    globals: BTreeSet<&'a str>,
    constructors: BTreeMap<&'a str, BTreeSet<&'a str>>,
}

struct References<'a, 'scope> {
    scope: &'scope DependencyScope<'a>,
    names: BTreeSet<String>,
    bindings: Vec<&'a str>,
}

impl<'a> References<'a, '_> {
    fn name(&mut self, name: &[String]) {
        if let [name] = name {
            if self.bindings.contains(&name.as_str()) {
                return;
            }
            if self.scope.globals.contains(name.as_str()) {
                self.names.insert(name.clone());
                return;
            }
        }
        self.constructor(name);
    }

    fn constructor(&mut self, name: &[String]) {
        match name {
            [name] => {
                if let Some(owners) = self.scope.constructors.get(name.as_str()) {
                    self.names
                        .extend(owners.iter().map(|owner| (*owner).to_owned()));
                }
            }
            [owner, ..] if self.scope.globals.contains(owner.as_str()) => {
                self.names.insert(owner.clone());
            }
            _ => {}
        }
    }

    fn binders(&mut self, binders: &'a [Binder]) {
        for binder in binders {
            self.term(&binder.ty);
            self.bindings.push(&binder.name);
        }
    }

    fn branches<T>(&mut self, arms: &'a [Arm<T>], body: impl Fn(&mut Self, &'a T)) {
        let saved = self.bindings.len();
        for arm in arms {
            if let Pattern::Constructor(name, bindings) = &arm.pattern {
                self.constructor(name);
                self.bindings.extend(bindings.iter().map(String::as_str));
            }
            body(self, &arm.body);
            self.bindings.truncate(saved);
        }
    }

    fn term(&mut self, term: &'a Term) {
        match term {
            Term::Name(name) => self.name(name),
            Term::Nat(_) | Term::Bool(_) => {}
            Term::Call(function, args) => {
                self.term(function);
                for arg in args {
                    self.term(arg);
                }
            }
            Term::Binary(_, left, right)
            | Term::Annotate(left, right)
            | Term::Pair(left, right) => {
                self.term(left);
                self.term(right);
            }
            Term::Forall(binder, body)
            | Term::Exists(binder, body)
            | Term::Lambda(binder, body) => {
                self.term(&binder.ty);
                self.bindings.push(&binder.name);
                self.term(body);
                self.bindings.pop();
            }
            Term::Let(binder, value, body) => {
                self.term(&binder.ty);
                self.term(value);
                self.bindings.push(&binder.name);
                self.term(body);
                self.bindings.pop();
            }
            Term::If(condition, yes, no) => {
                self.term(condition);
                self.term(yes);
                self.term(no);
            }
            Term::Match(value, arms) => {
                self.term(value);
                self.branches(arms, Self::term);
            }
        }
    }

    fn proof(&mut self, proof: &'a Proof) {
        match proof {
            Proof::Refl | Proof::Goal => {}
            Proof::Exact(term) => self.term(term),
            Proof::Intro(binding, body) => {
                self.bindings.push(binding);
                self.proof(body);
                self.bindings.pop();
            }
            Proof::Split(left, right) => {
                self.proof(left);
                self.proof(right);
            }
            Proof::Witness(term, body) | Proof::Rewrite(term, body) | Proof::Change(term, body) => {
                self.term(term);
                self.proof(body);
            }
            Proof::Branch(condition, yes_name, yes, no_name, no) => {
                self.term(condition);
                for (name, branch) in [(yes_name, yes), (no_name, no)] {
                    self.bindings.push(name);
                    self.proof(branch);
                    self.bindings.pop();
                }
            }
            Proof::Unfold(name, body) => {
                self.name(name);
                self.proof(body);
            }
            Proof::Cases(_, arms) | Proof::Induction(_, arms) => self.branches(arms, Self::proof),
            Proof::Left(body) | Proof::Right(body) => self.proof(body),
        }
    }
}
