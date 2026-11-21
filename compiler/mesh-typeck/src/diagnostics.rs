//! Type errors as diagnostics: each error's catalog code, and where and how
//! it is reported. Every sink (the terminal, JSON, the language server)
//! renders the `Diagnostic` this produces.

use mesh_common::span::Span;
use mesh_diagnostics::{Applicability, Code, Diagnostic, SourceFile};
use rowan::TextRange;

use crate::error::{ConstraintOrigin, TypeError};
use crate::ty::Ty;

impl TypeError {
    /// The catalog code shared by terminal, JSON and LSP type diagnostics.
    pub fn code(&self) -> Code {
        match self {
            TypeError::Mismatch { .. } => Code::TypeMismatch,
            TypeError::InfiniteType { .. } => Code::InfiniteType,
            TypeError::ArityMismatch { .. } | TypeError::TupleParameterSplit { .. } => {
                Code::ArityMismatch
            }
            TypeError::UnboundVariable { .. } => Code::UnboundVariable,
            TypeError::NotAFunction { .. } => Code::NotAFunction,
            TypeError::TraitNotSatisfied { .. } => Code::TraitNotSatisfied,
            TypeError::MissingTraitMethod { .. } => Code::MissingTraitMethod,
            TypeError::TraitMethodSignatureMismatch { .. } => Code::TraitMethodSignatureMismatch,
            TypeError::MissingField { .. }
            | TypeError::UnknownField { .. }
            | TypeError::NoSuchField { .. } => Code::InvalidField,
            TypeError::NoSuchMethod { .. } => Code::NoSuchMethod,
            TypeError::ManualContinuityPromotionDisabled { .. } => Code::ManualContinuityPromotion,
            TypeError::UnknownVariant { .. } => Code::UnknownVariant,
            TypeError::OrPatternBindingMismatch { .. } => Code::OrPatternBindingMismatch,
            TypeError::NonExhaustiveMatch { .. } => Code::NonExhaustiveMatch,
            TypeError::RedundantArm { .. } => Code::RedundantArm,
            TypeError::NonExhaustiveClauses { .. } => Code::NonExhaustiveClauses,
            TypeError::SendTypeMismatch { .. } => Code::MessageTypeMismatch,
            TypeError::SelfOutsideActor { .. } => Code::SelfOutsideActor,
            TypeError::MonitorOutsideActor { .. } => Code::MonitorOutsideActor,
            TypeError::SpawnNonFunction { .. } => Code::SpawnNonFunction,
            TypeError::ReceiveOutsideActor { .. } => Code::ReceiveOutsideActor,
            TypeError::InvalidChildStart { .. } => Code::InvalidChildStart,
            TypeError::InvalidStrategy { .. } => Code::UnknownSupervisionStrategy,
            TypeError::InvalidRestartType { .. } => Code::InvalidRestartType,
            TypeError::InvalidShutdownValue { .. } => Code::InvalidShutdownValue,
            TypeError::CatchAllNotLast { .. } => Code::CatchAllNotLast,
            TypeError::NonConsecutiveClauses { .. } => Code::NonConsecutiveClauses,
            TypeError::NonFirstClauseAnnotation { .. } => Code::NonFirstClauseAnnotation,
            TypeError::DuplicateImpl { .. } => Code::DuplicateImpl,
            TypeError::AmbiguousMethod { .. } => Code::AmbiguousMethod,
            TypeError::UnsupportedDerive { .. } => Code::UnsupportedDerive,
            TypeError::MissingDerivePrerequisite { .. } => Code::MissingDerivePrerequisite,
            TypeError::BreakOutsideLoop { .. } => Code::BreakOutsideLoop,
            TypeError::ContinueOutsideLoop { .. } => Code::ContinueOutsideLoop,
            TypeError::ImportModuleNotFound { .. } => Code::ModuleNotFound,
            TypeError::ImportNameNotFound { .. } => Code::ImportNameNotFound,
            TypeError::PrivateItem { .. } => Code::PrivateItem,
            TypeError::HttpClusteredInvalidArguments { .. } => Code::ClusteredRouteInvalidArguments,
            TypeError::HttpClusteredPrivateHandler { .. } => Code::ClusteredRoutePrivateHandler,
            TypeError::HttpClusteredOutsideRouteHandlerPosition { .. } => {
                Code::ClusteredRouteMisplaced
            }
            TypeError::HttpClusteredConflictingReplicationCount { .. } => {
                Code::ClusteredRouteConflictingCount
            }
            TypeError::HttpClusteredImportedOriginMissing { .. } => {
                Code::ClusteredRouteOriginMissing
            }
            TypeError::TryIncompatibleReturn { .. } => Code::TryIncompatibleReturn,
            TypeError::TryOnNonResultOption { .. } => Code::TryOnNonResult,
            TypeError::NonSerializableField { .. } => Code::NonSerializableField,
            TypeError::NonMappableField { .. } => Code::NonMappableField,
            TypeError::MissingAssocType { .. } => Code::MissingAssociatedType,
            TypeError::ExtraAssocType { .. } => Code::ExtraAssociatedType,
            TypeError::UnresolvedAssocType { .. } => Code::UnresolvedAssociatedType,
            TypeError::SlotPipeOutOfRange { .. } => Code::SlotPipeOutOfRange,
            TypeError::UndefinedType { .. } => Code::UndefinedAliasTarget,
            TypeError::NativeDeclarationInvalid { .. } => Code::InvalidNativeDeclaration,
            TypeError::ExportDeclarationInvalid { .. } => Code::InvalidExportDeclaration,
            TypeError::ResourceViolation { .. } => Code::ResourceOwnershipViolation,
            TypeError::InvalidLetPattern { .. } => Code::InvalidLetPattern,
            TypeError::InvalidPassThroughArm { .. } => Code::InvalidPassThroughArm,
            TypeError::DuplicateBinding { .. } => Code::DuplicateBinding,
            TypeError::DuplicateField { .. } => Code::DuplicateField,
            TypeError::NotAStruct { .. } => Code::NotAStruct,
            TypeError::UnderivableField { .. } => Code::UnderivableFunctionField,
            TypeError::DuplicateVariant { .. } => Code::DuplicateVariant,
            TypeError::CyclicAlias { .. } => Code::CyclicAlias,
            TypeError::UnboundedTypeParam { .. } => Code::UnboundedTypeParameter,
            TypeError::AmbiguousDefault { .. } => Code::AmbiguousDefault,
            TypeError::AmbiguousImplMethod { .. } => Code::AmbiguousImplMethod,
            TypeError::AmbiguousStaticMethod { .. } => Code::AmbiguousStaticMethod,
            TypeError::RigidTypeParam { .. } => Code::RigidTypeParameter,
            TypeError::DuplicateDefinition { .. } => Code::DuplicateDefinition,
            TypeError::UnknownType { .. } => Code::UnknownType,
            TypeError::UnknownFieldOwner { .. } => Code::UnknownFieldOwner,
            TypeError::UnknownInterface { .. } => Code::UnknownInterface,
            TypeError::InvalidLiteral { .. } => Code::InvalidLiteral,
            TypeError::InvalidConcat { .. } => Code::InvalidConcat,
            TypeError::NoSuchModuleFunction { .. } => Code::NoSuchModuleFunction,
            TypeError::OverloadedFunctionValue { .. } => Code::OverloadedFunctionValue,
            TypeError::GenericImplTarget { .. } => Code::GenericImplTarget,
            TypeError::AssertReceiveOutsideTest { .. } => Code::AssertReceiveOutsideTest,
            TypeError::IndexingUnsupported { .. } => Code::IndexingUnsupported,
            TypeError::ActorMessageTypeUnknown { .. } => Code::ActorMessageTypeUnknown,
            TypeError::TopLevelLet { .. } => Code::TopLevelLet,
            TypeError::ModuleNotImported { .. } => Code::ModuleNotImported,
            TypeError::UntypedMethodParam { .. } => Code::UntypedMethodParameter,
            TypeError::TypeNotValue { .. } => Code::TypeNotValue,
            TypeError::NestedDefinition { .. } => Code::NestedDefinition,
            TypeError::TypeArgumentCount { .. } => Code::TypeArgumentCount,
            TypeError::TopLevelStatement { .. } => Code::TopLevelStatement,
            TypeError::MethodReturnUnknown { .. } => Code::MethodReturnUnknown,
            TypeError::SpawnByName { .. } => Code::SpawnComputed,
            TypeError::SpawnNotActor { .. } => Code::SpawnNotActor,
            TypeError::DecodeTypeUnknown { .. } => Code::DecodeTypeUnknown,
            TypeError::UnderivableFieldType { .. } => Code::UnderivableFieldType,
            TypeError::PlaintextViolation { .. } => Code::PlaintextViolation,
            TypeError::JobsStartInvalid { .. } => Code::InvalidJobsStart,
            TypeError::InvalidIndex { .. } => Code::InvalidNatIndex,
            TypeError::InvalidIndexedType { .. } => Code::InvalidIndexedType,
            TypeError::ReservedProofName { .. } => Code::ReservedProofName,
            TypeError::MissingEmbeddedProof { .. } => Code::MissingEmbeddedProof,
            TypeError::DuplicateProofValue { .. } => Code::DuplicateProofValue,
            TypeError::InvalidProposition { .. } => Code::InvalidProposition,
            TypeError::LogicSyntax { error } => error.code,
        }
    }

    /// The error as a diagnostic in `file`, the source that was checked.
    ///
    /// Its message is the error's `Display` text, the headline every format
    /// shows, and its primary span is [`TypeError::span`]. An error with no
    /// span (a constraint of the builtins) is about the whole file. Without
    /// a repair for this instance, the catalog's stands.
    pub fn to_diagnostic(&self, file: &SourceFile) -> Diagnostic {
        let diagnostic = place(
            self,
            Diagnostic::in_file(self.code(), file, self.to_string()),
            file,
        );
        if mentions_plaintext(self) && !matches!(self, TypeError::PlaintextViolation { .. }) {
            diagnostic.with_note(PLAINTEXT_EXITS)
        } else {
            diagnostic
        }
    }
}

fn span_of(range: TextRange) -> Span {
    Span::new(range.start().into(), range.end().into())
}

/// `d` at `origin`'s span, labelled `label`; about the whole file when the
/// origin has none.
fn at_origin(d: Diagnostic, origin: &ConstraintOrigin, label: impl Into<String>) -> Diagnostic {
    match origin.span() {
        Some(span) => d.at(span_of(span), label),
        None => d,
    }
}

/// "did you mean `close`?", with the edit that makes the change when the
/// text at `span` is `name` as written.
fn did_you_mean(
    d: Diagnostic,
    file: &SourceFile,
    span: TextRange,
    name: &str,
    close: &str,
) -> Diagnostic {
    let span = span_of(span);
    let d = d.with_repair(format!("did you mean `{close}`?"));
    let written = file
        .text()
        .and_then(|text| text.get(span.start as usize..span.end as usize));
    if written == Some(name) {
        d.with_edit(span, close, Applicability::MaybeIncorrect)
    } else {
        d
    }
}

/// `d` placed at `error`'s spans, with the labels, repair and notes this
/// instance has.
fn place(error: &TypeError, d: Diagnostic, file: &SourceFile) -> Diagnostic {
    match error {
        TypeError::Mismatch {
            expected,
            found,
            origin,
        } => {
            let (expected, found) = (&expected.with_holes(), &found.with_holes());
            let d = match origin {
                ConstraintOrigin::IfBranches {
                    if_span,
                    then_span,
                    else_span,
                } => d
                    .at(span_of(*if_span), "")
                    .with_label(span_of(*then_span), format!("expected {expected}"))
                    .with_label(span_of(*else_span), format!("found {found}")),
                ConstraintOrigin::Annotation { annotation_span } => d.at(
                    span_of(*annotation_span),
                    format!("expected {expected} from annotation"),
                ),
                ConstraintOrigin::FnArg {
                    call_site,
                    param_idx,
                } => d.at(
                    span_of(*call_site),
                    format!(
                        "argument {} has type {found}, expected {expected}",
                        param_idx + 1
                    ),
                ),
                ConstraintOrigin::Return {
                    return_span,
                    fn_span,
                } => d
                    .at(span_of(*return_span), format!("returns {found}"))
                    .with_label(
                        span_of(*fn_span),
                        format!("return type declared as {expected}"),
                    ),
                ConstraintOrigin::Assignment { lhs_span, rhs_span } => d
                    .at(span_of(*lhs_span), format!("expected {expected}"))
                    .with_label(span_of(*rhs_span), format!("found {found}")),
                ConstraintOrigin::Pattern { pattern_span } => d.at(
                    span_of(*pattern_span),
                    format!("this pattern matches {found}, but the value is {expected}"),
                ),
                _ => at_origin(d, origin, format!("expected {expected}, found {found}")),
            };
            match fix_suggestion(expected, found) {
                Some(fix) if !matches!(origin, ConstraintOrigin::Pattern { .. }) => {
                    d.with_repair(fix)
                }
                _ => d,
            }
        }

        TypeError::InfiniteType { origin, .. } => at_origin(d, origin, "recursive type here")
            .with_repair("a value cannot have a type that refers to itself"),

        TypeError::ArityMismatch {
            expected,
            found,
            origin,
        } => {
            let d = at_origin(d, origin, format!("expected {expected} argument(s)"));
            match origin {
                ConstraintOrigin::PipePlaceholder {
                    placeholder_slot, ..
                } => {
                    let repair = if *placeholder_slot == 1 {
                        "Keep `|>` and remove `_` from the call to use the piped value as argument 1."
                            .to_string()
                    } else {
                        format!(
                            "Use `|{placeholder_slot}>` and remove `_` from the call \
                             to pass the value as argument {placeholder_slot}."
                        )
                    };
                    d.with_repair(format!(
                        "`|>` passes the piped value as the first argument. The call counts `_` as a written argument. \
                         {repair} See `meshc guide syntax`."
                    ))
                }
                _ if expected > found => {
                    d.with_repair(format!("missing {} argument(s)", expected - found))
                }
                _ => d.with_repair(format!("{} extra argument(s)", found - expected)),
            }
        }

        TypeError::TupleParameterSplit { elements, origin } => {
            let names: Vec<String> = (0..*elements)
                .map(|i| ((b'a' + (i % 26) as u8) as char).to_string())
                .collect();
            at_origin(d, origin, format!("takes {elements} arguments, not a tuple")).with_repair(
                format!(
                    "`fn ({}) -> ...` takes {elements} arguments; to take the tuple apart, \
                     put its pattern in parentheses: `fn (({})) -> ... end`",
                    names.join(", "),
                    names.join(", ")
                ),
            )
        }

        TypeError::UnboundVariable {
            name,
            span,
            suggestion,
        } => {
            let d = d.at(span_of(*span), "not found in this scope");
            if matches!(name.as_str(), "null" | "undefined") {
                d.with_repair(format!(
                    "Mesh has no `{name}`: a value that may be absent is an `Option` \
                     (`None`, or `Some(value)`)"
                ))
            } else if let Some(close) = suggestion {
                did_you_mean(d, file, *span, name, close)
            } else {
                d
            }
        }

        TypeError::NotAFunction { ty, span } => d
            .at(
                span_of(*span),
                format!("{} is not a function", ty.with_holes()),
            )
            .with_repair("only functions can be called; remove the parentheses to use the value"),

        TypeError::TraitNotSatisfied {
            ty,
            trait_name,
            builtin,
            origin,
        } => at_origin(
            d,
            origin,
            format!("{} does not satisfy {}", ty.with_holes(), trait_name),
        )
        .with_repair(if trait_name == "Json" {
            "JSON holds Int, Float, Bool, String, tuples, and Option, List and \
             Map<String, _> of them; a struct or sum type gets it with `deriving(Json)`"
                .to_string()
        } else if is_named_type(ty)
            && !builtin
            && matches!(
                trait_name.as_str(),
                "Eq" | "Ord" | "Display" | "Debug" | "Hash"
            )
        {
            format!(
                "add `deriving({trait_name})` to the definition of `{ty}`, \
                 or `impl {trait_name} for {ty} do ... end`"
            )
        } else if matches!(trait_name.as_str(), "Eq" | "Ord" | "Display")
            && (matches!(ty, Ty::Tuple(_))
                || matches!(
                    ty.con_name(),
                    Some("Option" | "Result" | "List" | "Map" | "Set")
                ))
        {
            let what = match ty.con_name() {
                Some(name) => format!("`{name}`"),
                None => "a tuple".to_string(),
            };
            format!(
                "{what} has `{trait_name}` only when what it holds does, and here it holds \
                 something without it"
            )
        } else if trait_name == "Add" && matches!(ty.con_name(), Some("String" | "List")) {
            "join strings with `<>`, and lists with `++`".to_string()
        } else if is_named_type(ty) {
            format!("add `impl {} for {} do ... end`", trait_name, ty)
        } else {
            "only a named type without type parameters can have an `impl`".to_string()
        }),

        TypeError::UnboundedTypeParam {
            param,
            trait_name,
            origin,
        } => at_origin(d, origin, format!("this needs {param}: {trait_name}")).with_repair(
            format!(
                "add `where {param}: {trait_name}` to the function, so every call is checked for it"
            ),
        ),

        TypeError::MissingTraitMethod {
            trait_name,
            method_name,
            span,
            ..
        } => d
            .at(span_of(*span), format!("missing `{}`", method_name))
            .with_repair(format!(
                "add `fn {}` as `{}` declares it to the impl block",
                method_name, trait_name
            )),

        TypeError::TraitMethodSignatureMismatch {
            trait_name,
            expected,
            found,
            span,
            ..
        } => d.at(
            span_of(*span),
            format!(
                "`{}` declares {}, this is {}",
                trait_name,
                expected.with_holes(),
                found.with_holes()
            ),
        ),

        TypeError::MissingField {
            field_name, span, ..
        } => d
            .at(span_of(*span), format!("field `{}` is required", field_name))
            .with_repair(format!("add `{}: <value>`", field_name)),

        TypeError::UnknownField {
            struct_name,
            field_name,
            span,
        } => d.at(
            span_of(*span),
            format!("`{}` has no field `{}`", struct_name, field_name),
        ),

        TypeError::NoSuchField {
            field_name, span, ..
        } => d.at(span_of(*span), format!("no field `{}`", field_name)),

        TypeError::NoSuchMethod {
            ty,
            method_name,
            span,
        } => d
            .at(span_of(*span), format!("method `{}` not found", method_name))
            .with_repair(format!(
                "type `{}` has no trait impl providing `{}`",
                ty, method_name
            )),

        TypeError::ManualContinuityPromotionDisabled { span } => d
            .at(
                span_of(*span),
                "manual authority changes are no longer part of the Mesh surface",
            )
            .with_repair(
                "failover is automatic-only now; use `Continuity.authority_status()` to inspect authority state",
            ),

        TypeError::UnknownVariant {
            name,
            span,
            suggestion,
            cons_tail,
        } => {
            let d = d.at(span_of(*span), "not a known variant");
            if *cons_tail {
                d.with_repair(format!(
                    "`::` in a pattern splits a list into its head and tail, so `{name}` was read \
                     as a variant; a pattern takes no type annotation"
                ))
            } else if let Some(close) = suggestion {
                did_you_mean(d, file, *span, name, close)
            } else {
                d
            }
        }

        TypeError::OrPatternBindingMismatch {
            list_tail, span, ..
        } => d
            .at(span_of(*span), "alternatives must bind the same variables")
            .with_repair(if *list_tail {
                "`|` in a list pattern separates alternatives; a list's tail is written \
                 `first :: rest` (`a :: b :: rest` after two elements)"
            } else {
                "all alternatives in an or-pattern must bind the same set of variable names"
            }),

        TypeError::NonExhaustiveMatch {
            missing_patterns,
            span,
            ..
        } => {
            let d = d.at(
                span_of(*span),
                format!("missing: {}", missing_patterns.join(", ")),
            );
            if missing_patterns.iter().any(|pattern| pattern == "_") {
                return d.with_repair("add a wildcard `_` arm to handle the remaining cases");
            }
            let patterns = missing_patterns
                .iter()
                .map(|pattern| format!("`{pattern}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let arms = if missing_patterns.len() == 1 {
                "an arm"
            } else {
                "arms"
            };
            d.with_repair(format!(
                "add {arms} for {patterns}. Use a wildcard `_` arm for an intentional default \
                 that also covers future variants."
            ))
        }

        TypeError::NonExhaustiveClauses {
            missing_patterns,
            span,
            ..
        } => d
            .at(
                span_of(*span),
                format!("missing: {}", missing_patterns.join(", ")),
            )
            .with_repair("add a clause for the missing patterns: a call no clause matches panics"),

        TypeError::RedundantArm { span, .. } => d
            .at(span_of(*span), "this arm is unreachable")
            .with_repair("remove this arm or reorder the match"),

        TypeError::SendTypeMismatch {
            expected,
            found,
            span,
        } => {
            let (expected, found) = (&expected.with_holes(), &found.with_holes());
            d.at(
                span_of(*span),
                format!("expected {}, found {}", expected, found),
            )
            .with_repair(format!("this Pid accepts messages of type {}", expected))
        }

        TypeError::SelfOutsideActor { span } => d.at(
            span_of(*span),
            "self() is only available inside an actor block",
        ),

        TypeError::MonitorOutsideActor { span } => d
            .at(
                span_of(*span),
                "a monitor delivers its message to the actor that sets it up",
            )
            .with_repair("call `Process.monitor` or `Node.monitor` inside an actor block"),

        TypeError::SpawnNonFunction { found, span } => d.at(
            span_of(*span),
            format!("expected a function, found {}", found.with_holes()),
        ),

        TypeError::UnderivableFieldType {
            trait_name,
            type_name,
            field_ty,
            span,
            ..
        } => {
            let field_ty = field_ty.with_holes();
            d.at(span_of(*span), format!("this has no `{trait_name}`"))
                .with_repair(format!(
                    "derive or implement `{trait_name}` for `{field_ty}`, or leave `{trait_name}` out \
                     of `{type_name}`'s deriving list"
                ))
        }

        TypeError::DecodeTypeUnknown { ty, span } => d
            .at(span_of(*span), "nothing fixes what this decodes")
            .with_repair(format!(
                "annotate the result, as `let r :: Result<{ty}, String> = ...` with the `_` \
                 filled in"
            )),

        TypeError::SpawnNotActor { span } => d
            .at(span_of(*span), "a function, not an actor")
            .with_repair(
                "define what runs in the new process as `actor name(...) do ... end`, \
                 or call the function",
            ),

        TypeError::SpawnByName { span } => d
            .at(span_of(*span), "a function worked out here, not named")
            .with_repair(
                "name the actor or function, `spawn(worker, ...)`; to choose one at run time, \
                 spawn each by name where it is chosen",
            ),

        TypeError::ReceiveOutsideActor { span } => d
            .at(
                span_of(*span),
                "receive is only available inside an actor block",
            )
            .with_repair("move this receive expression into an actor block"),

        TypeError::InvalidChildStart { found, span, .. } => d
            .at(
                span_of(*span),
                format!("expected Pid<M>, found {}", found.with_holes()),
            )
            .with_repair("the start function must call spawn() and return a Pid"),

        TypeError::InvalidStrategy { span, .. } => d.at(
            span_of(*span),
            "expected one_for_one, one_for_all, rest_for_one, or simple_one_for_one",
        ),

        TypeError::InvalidRestartType { span, .. } => d.at(
            span_of(*span),
            "expected permanent, transient, or temporary",
        ),

        TypeError::InvalidShutdownValue { span, .. } => d.at(
            span_of(*span),
            "expected a positive integer or brutal_kill",
        ),

        TypeError::CatchAllNotLast { span, .. } => d.at(
            span_of(*span),
            "clauses after a catch-all are unreachable",
        ),

        TypeError::NonConsecutiveClauses {
            first_span,
            second_span,
            ..
        } => d
            .at(span_of(*second_span), "non-consecutive redefinition here")
            .with_label(span_of(*first_span), "first definition here"),

        TypeError::NonFirstClauseAnnotation { span, .. } => d.at(
            span_of(*span),
            "only the first clause should have this annotation",
        ),

        TypeError::DuplicateImpl {
            first_impl, span, ..
        } => d
            .at(span_of(*span), first_impl.to_string())
            .with_repair("remove one of the conflicting impl blocks"),

        TypeError::AmbiguousMethod {
            method_name,
            candidate_traits,
            span,
            ..
        } => {
            let suggestions: Vec<String> = candidate_traits
                .iter()
                .map(|t| format!("{}.{}(value)", t, method_name))
                .collect();
            d.at(
                span_of(*span),
                format!("multiple traits provide `{}`", method_name),
            )
            .with_repair(format!(
                "use qualified syntax: {}",
                suggestions.join(" or ")
            ))
        }

        TypeError::UnsupportedDerive {
            trait_name, span, ..
        } => d
            .at(
                span_of(*span),
                format!("`{}` cannot be derived here", trait_name),
            )
            .with_repair(
                "structs derive Eq, Ord, Display, Debug, Hash, Json, Row, and Schema; sum types all but Row and Schema",
            ),

        TypeError::MissingDerivePrerequisite {
            trait_name,
            requires,
            span,
            ..
        } => d
            .at(
                span_of(*span),
                format!(
                    "`{}` requires `{}` for its implementation",
                    trait_name, requires
                ),
            )
            .with_repair(format!(
                "add `{}` to the deriving list: deriving({}, {})",
                requires, requires, trait_name
            )),

        TypeError::BreakOutsideLoop { span } => d
            .at(
                span_of(*span),
                "`break` can only be used inside a `while` or `for` loop",
            )
            .with_repair("move this `break` inside a loop body"),

        TypeError::ContinueOutsideLoop { span } => d
            .at(
                span_of(*span),
                "`continue` can only be used inside a `while` or `for` loop",
            )
            .with_repair("move this `continue` inside a loop body"),

        // The headline names the module and any near miss, or the name and
        // what the module exports.
        TypeError::ImportModuleNotFound { span, .. } => {
            d.at(span_of(*span), "no module has this name")
        }

        TypeError::ImportNameNotFound { span, .. } => d.at(span_of(*span), "not exported"),

        TypeError::PrivateItem {
            module_name,
            name,
            span,
        } => d
            .at(span_of(*span), "private to its module")
            .with_repair(format!(
                "add `pub` to `{}` in module `{}` to make it accessible",
                name, module_name
            )),

        TypeError::HttpClusteredInvalidArguments { span, .. } => d
            .at(span_of(*span), "this wrapper")
            .with_repair(
                "use `HTTP.clustered(handler)` or `HTTP.clustered(<int>, handler)` with a public top-level route handler reference",
            ),

        TypeError::JobsStartInvalid { span, .. } => d.at(span_of(*span), "here"),

        TypeError::HttpClusteredPrivateHandler { handler_name, span } => d
            .at(
                span_of(*span),
                format!(
                    "`{}` is private and cannot cross the clustered route boundary",
                    handler_name
                ),
            )
            .with_repair(format!(
                "add `pub` to `{}` or remove `HTTP.clustered(...)`",
                handler_name
            )),

        TypeError::HttpClusteredOutsideRouteHandlerPosition { span } => d.at(
            span_of(*span),
            "use this wrapper directly as the handler argument to `HTTP.route(...)` or `HTTP.on_*(...)`",
        ),

        TypeError::HttpClusteredConflictingReplicationCount {
            runtime_name,
            first_count,
            current_count,
            first_span,
            span,
        } => d
            .at(
                span_of(*span),
                format!(
                    "conflicting replication count {} for `{}`",
                    current_count, runtime_name
                ),
            )
            .with_label(
                span_of(*first_span),
                format!(
                    "`{}` was first declared here with replication count {}",
                    runtime_name, first_count
                ),
            )
            .with_repair("keep one replication count per clustered route handler runtime name"),

        TypeError::HttpClusteredImportedOriginMissing { handler_name, span } => d
            .at(
                span_of(*span),
                format!(
                    "cannot determine the defining module for imported handler `{}`",
                    handler_name
                ),
            )
            .with_note(
                "imported bare handlers must preserve their defining module so clustered route lowering can keep the real runtime name",
            ),

        TypeError::TryIncompatibleReturn {
            operand_ty,
            fn_return_ty,
            span,
        } => d.at(span_of(*span), "cannot use `?` here").with_note(format!(
            "cannot propagate `{}` from a function returning `{}`; Result errors must match or have a From conversion, and Option requires an Option return type",
            operand_ty.with_holes(),
            fn_return_ty.with_holes()
        )),

        TypeError::TryOnNonResultOption { operand_ty, span } => d
            .at(
                span_of(*span),
                format!(
                    "type `{}` is not `Result` or `Option`",
                    operand_ty.with_holes()
                ),
            )
            .with_repair(
                "the `?` operator can only be used on `Result<T, E>` or `Option<T>` values",
            ),

        TypeError::NonSerializableField {
            field_type, span, ..
        } => d
            .at(
                span_of(*span),
                format!("`{}` is not serializable", field_type),
            )
            .with_repair(format!(
                "type `{}` does not derive Json; add `deriving(Json)` to its definition, or use a serializable type (Int, Float, Bool, String, a tuple, Option<T>, List<T>, Map<String, V>)",
                field_type
            )),

        TypeError::NonMappableField {
            field_type, span, ..
        } => d
            .at(
                span_of(*span),
                format!("`{}` is not row-mappable", field_type),
            )
            .with_repair(
                "only Int, Float, Bool, String, and Option<T> fields are supported for deriving(Row)",
            ),

        TypeError::MissingAssocType {
            assoc_name, span, ..
        } => d
            .at(
                span_of(*span),
                format!("missing `type {} = ...`", assoc_name),
            )
            .with_repair(format!(
                "add `type {} = <ConcreteType>` to the impl block",
                assoc_name
            )),

        TypeError::ExtraAssocType {
            trait_name,
            assoc_name,
            span,
            ..
        } => d.at(
            span_of(*span),
            format!("`{}` is not declared by `{}`", assoc_name, trait_name),
        ),

        TypeError::UnresolvedAssocType { assoc_name, span } => d
            .at(span_of(*span), "not declared")
            .with_repair(format!(
                "declare it in the interface with `type {assoc_name}`, and bind it in each impl"
            )),

        TypeError::SlotPipeOutOfRange {
            slot, arity, span, ..
        } => {
            let d = d.at(
                span_of(*span),
                format!("slot {} exceeds function arity {}", slot, arity),
            );
            if *arity <= 1 {
                d.with_repair("use |> to pipe as the first argument")
            } else {
                d.with_repair(format!(
                    "use |> to pipe as the first argument, or |2>...|{}> for other positions",
                    arity
                ))
            }
        }

        TypeError::UndefinedType {
            target_name, span, ..
        } => d
            .at(span_of(*span), format!("`{}` is not defined", target_name))
            .with_repair(format!(
                "define `{}` as a struct, sum type, or type alias before using it here",
                target_name
            )),

        TypeError::NativeDeclarationInvalid { span, .. } => d
            .at(span_of(*span), "this declaration")
            .with_repair(
                "use a public, fully annotated, non-generic signature and a C identifier symbol",
            ),

        TypeError::ExportDeclarationInvalid { span, .. } => d
            .at(span_of(*span), "this declaration")
            .with_repair(
                "use `@export(\"c_symbol\") pub fn name(request :: Bytes) -> Bytes!String`",
            ),

        TypeError::InvalidLetPattern { span, .. } => d
            .at(span_of(*span), "this pattern")
            .with_repair(
                "use only lowercase names, `_`, and tuple and struct patterns; use `case` for refutable patterns",
            ),

        TypeError::DuplicateField { span, .. } => d
            .at(span_of(*span), "given again here")
            .with_repair("give each field one value"),

        TypeError::UnderivableField {
            trait_name, span, ..
        } => d
            .at(
                span_of(*span),
                "functions cannot be compared, hashed or shown",
            )
            .with_repair(format!("remove `{trait_name}` from the deriving list")),

        TypeError::DuplicateVariant { variant, span, .. } => d
            .at(
                span_of(*span),
                format!("`{variant}` would name either type's variant"),
            )
            .with_repair("give one of the variants another name"),

        TypeError::AmbiguousImplMethod {
            method,
            candidates,
            found,
            span,
            by_argument,
            ..
        } => {
            let types = candidates
                .iter()
                .map(|ty| format!("`{ty}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let first = candidates
                .first()
                .map_or_else(|| "Int".to_string(), |ty| ty.to_string());
            let (label, repair) = match (found, by_argument) {
                (Some(_), false) => (
                    format!("they return {types}"),
                    format!("give the result a type: `let x :: {first} = value.{method}()`"),
                ),
                (None, false) => (
                    format!("its impls return {types}"),
                    format!("give the result a type: `let x :: {first} = value.{method}()`"),
                ),
                (Some(_), true) => (
                    format!("they take {types}"),
                    "convert the argument to one of those types first".to_string(),
                ),
                (None, true) => (
                    format!("its impls take {types}"),
                    format!("give the argument a type, such as `{first}`"),
                ),
            };
            d.at(span_of(*span), label).with_repair(repair)
        }

        TypeError::DuplicateDefinition { name, span, .. } => d
            .at(span_of(*span), "defined again here")
            .with_repair(format!("rename or remove one of the two `{name}`s")),

        TypeError::UnknownType { span, .. } => d
            .at(span_of(*span), "no type has this name")
            .with_repair("check the spelling, or define or import the type"),

        TypeError::NoSuchModuleFunction {
            module,
            available,
            span,
            ..
        } => {
            let d = d.at(span_of(*span), format!("not a function of `{module}`"));
            if available.is_empty() {
                d
            } else {
                d.with_repair(format!("`{module}` has {}", available.join(", ")))
            }
        }

        TypeError::AssertReceiveOutsideTest { span } => d
            .at(span_of(*span), "not in a test file")
            .with_repair("elsewhere, write the `receive ... after TIMEOUT -> ...` it stands for"),

        TypeError::GenericImplTarget { name, span } => d
            .at(span_of(*span), format!("`{name}` takes type parameters"))
            .with_repair(format!(
                "implement it for a type without type parameters, such as a struct \
                 holding the `{name}` you mean"
            )),

        TypeError::OverloadedFunctionValue {
            name,
            arities,
            span,
        } => {
            let counts = crate::error::arity_list(arities, " or ");
            let shown = arities.iter().copied().find(|&a| a > 0).unwrap_or(0);
            let params: Vec<String> = (0..shown).map(|i| format!("a{i}")).collect();
            let params = params.join(", ");
            d.at(span_of(*span), "each arity is its own function")
                .with_repair(format!(
                    "call it with {counts} arguments; as a value, use a closure \
                     that calls one: `fn {params} -> {name}({params}) end`"
                ))
        }

        TypeError::InvalidConcat { ty, span, .. } => d
            .at(
                span_of(*span),
                format!("these operands are `{}`", ty.with_holes()),
            )
            .with_repair("convert the values to strings first, e.g. with `\"${a}${b}\"`"),

        TypeError::TopLevelLet { name, span } => d
            .at(span_of(*span), "a module has no global bindings")
            .with_repair(format!(
                "move it into the function that uses it, or make it a function: `fn {name}() do ... end`"
            )),

        TypeError::TopLevelStatement { span } => d
            .at(span_of(*span), "nothing runs this")
            .with_repair(
                "a program runs `main` and what it calls: move this into `main` or the function \
                 that needs it",
            ),

        TypeError::ModuleNotImported { module, span, .. } => d
            .at(span_of(*span), "a module of this project")
            .with_repair(format!("add `import {module}` at the top of the file")),

        TypeError::ActorMessageTypeUnknown { actor, span } => d
            .at(span_of(*span), "no typed `Pid` reaches this actor")
            .with_repair(format!(
                "give the pid a message type where it is spawned, as `let pid :: Pid<Int> = spawn({actor})`, or use the messages in the actor as the type they are"
            )),

        TypeError::IndexingUnsupported { span } => d
            .at(span_of(*span), "no indexing syntax")
            .with_repair(
                "use `List.get(list, index)`, `Map.get(map, key)`, or `Tuple.nth(tuple, index)`",
            ),

        TypeError::InvalidLiteral { span, .. } => d.at(span_of(*span), "invalid literal"),

        TypeError::UnknownInterface { span, .. } => d
            .at(span_of(*span), "no interface has this name")
            .with_repair("check the spelling, or declare the interface or import its module"),

        TypeError::UnknownFieldOwner { span, .. } => d
            .at(span_of(*span), "nothing here fixes the type of this value")
            .with_repair("annotate the value's type, such as a parameter `p :: Point`"),

        TypeError::NestedDefinition { keyword, span } => d
            .at(span_of(*span), "inside a function")
            .with_repair(if *keyword == "fn" {
                "move it to the top level, or bind a closure: `let name = fn x -> ... end`"
            } else {
                "move it to the top level of the module"
            }),

        TypeError::TypeArgumentCount { span, .. } => {
            d.at(span_of(*span), "wrong number of type arguments")
        }

        TypeError::InvalidIndex { span, .. } => d.at(span_of(*span), "invalid Nat index"),

        TypeError::InvalidIndexedType { span, .. } => d.at(span_of(*span), "invalid indexed type"),

        TypeError::ReservedProofName { span } => {
            d.at(span_of(*span), "reserved for the type of proof values")
        }

        TypeError::MissingEmbeddedProof { theorem, span } => d
            .at(span_of(*span), "no `proof` of this theorem")
            .with_repair(format!(
                "write `proof {theorem} do ... end` in a `logic do` block of this module, or remove `pub` to keep `{theorem}` out of Mesh code"
            )),

        TypeError::DuplicateProofValue { name, first, span } => d
            .at(span_of(*span), "defined again here")
            .with_label(span_of(*first), "first defined here")
            .with_repair(format!(
                "rename one `{name}` and its `proof`, and update the calls that use it"
            )),

        TypeError::InvalidProposition { span, .. } => {
            d.at(span_of(*span), "has no proof type here")
        }

        // Its own lowering: the logic parser's labels and repair.
        TypeError::LogicSyntax { error } => error.to_diagnostic(file),

        TypeError::TypeNotValue {
            name,
            builtin,
            span,
        } => d
            .at(span_of(*span), "a type names no value")
            .with_repair(if *builtin {
                format!("call a function of its module, `{name}.function(...)`")
            } else {
                format!(
                    "build a value of it (`{name} {{ ... }}` for a struct, a variant for a sum type), or call one of its methods, `{name}.method(...)`"
                )
            }),

        TypeError::UntypedMethodParam { param, span, .. } => d
            .at(span_of(*span), "nothing fixes the type of this parameter")
            .with_repair(format!(
                "annotate it, `{param} :: Type`: a method is compiled once, for the type it is implemented for, not for each call"
            )),

        TypeError::MethodReturnUnknown { method, span, .. } => d
            .at(span_of(*span), "nothing fixes what this method returns")
            .with_repair(format!(
                "annotate its return type in the `impl` or the interface, `fn {method}(self) -> Type`"
            )),

        TypeError::RigidTypeParam { param, found, span } => d
            .at(span_of(*span), format!("`{param}` is declared here"))
            .with_repair(format!(
                "a generic function must work for every `{param}`: use `{}` in its signature instead, or keep `{param}` values as they are",
                found.with_holes()
            )),

        TypeError::AmbiguousStaticMethod {
            method,
            types,
            span,
        } => {
            let example = types.first().map_or("Type", String::as_str);
            d.at(span_of(*span), "which type's is meant?")
                .with_repair(format!("call it on the type: `{example}.{method}()`"))
        }

        TypeError::AmbiguousDefault { span } => d
            .at(span_of(*span), "nothing fixes this value's type")
            .with_repair("annotate it: `let x :: Int = default()`"),

        TypeError::CyclicAlias { span, .. } => d
            .at(span_of(*span), "expanding it never ends")
            .with_repair(
                "an alias names an existing type; use a struct or sum type for a recursive type",
            ),

        TypeError::NotAStruct { ty, span } => d
            .at(
                span_of(*span),
                if matches!(ty, Ty::Var(_)) {
                    "the type of this value is not known here"
                } else {
                    "not a struct"
                },
            )
            .with_repair("`Name { field: value }` and `%{value | field: new}` work on structs; annotate the value with its struct type if it has one"),

        TypeError::DuplicateBinding { span, .. } => d
            .at(span_of(*span), "each name in a pattern binds one value")
            .with_repair("rename one of them, or compare the values in a `when` guard"),

        TypeError::InvalidPassThroughArm { span, .. } => d
            .at(span_of(*span), "this arm")
            .with_repair("write the value after `->`: `pattern -> value`"),

        TypeError::PlaintextViolation { span, .. } => {
            d.at(span_of(*span), "here").with_repair(PLAINTEXT_EXITS)
        }

        TypeError::ResourceViolation { reason, span } => {
            // Across an actor, a job or into JSON it is the value itself
            // that cannot go, however it is moved.
            let crosses = reason.contains("cross") || reason.contains("transferred");
            d.at(span_of(*span), "here").with_repair(if crosses {
                "a resource stays with the actor that owns it: pass what it stands for \
                 instead, such as an id or the data it holds"
            } else {
                "move each resource once, or pass it to a direct `borrow` parameter"
            })
        }
    }
}

/// A type named with no type arguments, as a struct or `Int` is: one an
/// `impl` can be written for.
fn is_named_type(ty: &Ty) -> bool {
    match ty {
        Ty::Con(_) => true,
        Ty::App(con, args) => args.is_empty() && matches!(con.as_ref(), Ty::Con(_)),
        _ => false,
    }
}

/// How to turn a value of type `found` into the `expected` one, both as a
/// diagnostic shows them (`Ty::with_holes`).
fn fix_suggestion(expected: &Ty, found: &Ty) -> Option<&'static str> {
    let first_arg_of = |name: &str| match expected {
        Ty::App(con, args) if matches!(con.as_ref(), Ty::Con(c) if c.name == name) => args.first(),
        _ => None,
    };
    if first_arg_of("Option").is_some_and(|inner| could_be(inner, found)) {
        return Some("wrap in Some(...)");
    }
    if first_arg_of("Result").is_some_and(|ok| could_be(ok, found)) {
        return Some("wrap in Ok(...)");
    }
    match (expected.to_string().as_str(), found.to_string().as_str()) {
        ("Int", "Float") => Some("convert it with `Float.to_int(...)`"),
        ("Float", "Int") => Some("convert it with `Int.to_float(...)`"),
        ("String", "Int" | "Float") => Some("use to_string()"),
        // An argument converts on its own (`json_as_text`); nothing else does.
        ("String", "Json") => Some("encode it: `Json.encode(...)` is its JSON text"),
        ("Json", "String") => Some("parse it: `Json.parse(...)` reads JSON text into a Json"),
        ("Bool", _) => Some("expected a boolean expression"),
        _ => None,
    }
}

/// Whether `a` and `b`, as a diagnostic shows them, could be one type: the
/// same where neither has a hole (`_`, a part inference did not settle).
fn could_be(a: &Ty, b: &Ty) -> bool {
    let hole = |ty: &Ty| matches!(ty, Ty::Con(c) if c.name == "_");
    match (a, b) {
        _ if hole(a) || hole(b) => true,
        (Ty::App(..), Ty::App(..)) | (Ty::Fun(..), Ty::Fun(..)) | (Ty::Tuple(_), Ty::Tuple(_)) => {
            a.parts().count() == b.parts().count()
                && a.parts().zip(b.parts()).all(|(a, b)| could_be(a, b))
        }
        // `Point` is `Point` with no type arguments.
        _ => a.to_string() == b.to_string(),
    }
}

/// Where a `Plaintext` value may go, noted on every error about one.
const PLAINTEXT_EXITS: &str = "a `Plaintext` value leaves the program only through \
    `Crypto.aead_seal_plaintext`, `Crypto.hpke_seal_plaintext`, \
    `Plaintext.seal_for_storage`, an `@display` export, or \
    `declassify(value, \"reason\")`; work on it with `Plaintext.map`";

/// Whether `error` is about a `Plaintext` value: a plaintext violation, or
/// a type or trait error whose types name `Plaintext`.
fn mentions_plaintext(error: &TypeError) -> bool {
    let names = |ty: &Ty| ty.to_string().contains(crate::ty::PLAINTEXT);
    match error {
        TypeError::Mismatch {
            expected, found, ..
        }
        | TypeError::SendTypeMismatch {
            expected, found, ..
        } => names(expected) || names(found),
        TypeError::TraitNotSatisfied { ty, .. } => names(ty),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Errors with a place of their own are at it; some were reported as
    /// covering the whole file.
    #[test]
    fn diagnostics_point_at_the_error_not_the_whole_file() {
        let source = "fn f(0) = 0\nfn g() = 1\nfn f(n) = n\n";
        let file = SourceFile::new("m.mpl", source);
        let second = source.rfind("fn f").unwrap() as u32;
        let range = |start: u32, end: u32| rowan::TextRange::new(start.into(), end.into());
        let error = TypeError::NonConsecutiveClauses {
            fn_name: "f".to_string(),
            arity: 1,
            first_span: range(0, 11),
            second_span: range(second, second + 11),
        };
        let diagnostic = error.to_diagnostic(&file);
        assert_eq!(diagnostic.span(), Some(Span::new(second, second + 11)));
        assert_eq!(diagnostic.labels()[0].span(), Span::new(0, 11));
        // An error about no one place is about the whole file.
        let error = TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::Builtin,
        };
        let diagnostic = error.to_diagnostic(&file);
        assert_eq!(diagnostic.file(), Some(&file));
        assert_eq!(diagnostic.span(), None);
    }

    /// A misspelled name gets the closest one as its repair, and an edit to
    /// make it where the error's span is the name as written.
    #[test]
    fn a_near_miss_is_suggested_with_an_edit() {
        let source = "fn main() do\n  let count = 1\n  println(cuont)\nend\n";
        let file = SourceFile::new("m.mpl", source);
        let start = source.find("cuont").unwrap() as u32;
        let error = |name: &str, end: u32| TypeError::UnboundVariable {
            name: name.to_string(),
            span: rowan::TextRange::new(start.into(), end.into()),
            suggestion: Some("count".to_string()),
        };
        let diagnostic = error("cuont", start + 5).to_diagnostic(&file);
        assert_eq!(diagnostic.repair(), "did you mean `count`?");
        let edit = &diagnostic.edits()[0];
        assert_eq!(edit.span(), Span::new(start, start + 5));
        assert_eq!(edit.replacement(), "count");
        assert_eq!(edit.applicability(), Applicability::MaybeIncorrect);
        // A span that is not the name is no place to put the new one.
        let diagnostic = error("cuont", start + 6).to_diagnostic(&file);
        assert_eq!(diagnostic.repair(), "did you mean `count`?");
        assert!(diagnostic.edits().is_empty());
    }

    /// Without a repair of its own, an error has its code's.
    #[test]
    fn an_error_without_its_own_repair_has_the_catalogs() {
        let file = SourceFile::new("m.mpl", "fn main() do\nend\n");
        let error = TypeError::SelfOutsideActor {
            span: rowan::TextRange::new(0.into(), 2.into()),
        };
        let diagnostic = error.to_diagnostic(&file);
        assert_eq!(diagnostic.repair(), Code::SelfOutsideActor.info().repair);
        assert_eq!(diagnostic.message(), error.to_string());
    }

    /// A mismatch suggests how to turn the value found into the one
    /// expected. `Ok(...)` was not suggested for a `Result` whose value type
    /// has a comma in it (`Map<String, Int>`) or a part not settled yet.
    #[test]
    fn mismatches_suggest_the_conversion_they_need() {
        let con = |name: &str| Ty::Con(crate::ty::TyCon::new(name));
        let map = |k: Ty, v: Ty| Ty::map(k, v);
        let cases = [
            (Ty::option(Ty::int()), Ty::int(), Some("wrap in Some(...)")),
            (Ty::option(Ty::int()), Ty::string(), None),
            (
                Ty::result(map(Ty::string(), Ty::int()), Ty::string()),
                map(con("_"), con("_")),
                Some("wrap in Ok(...)"),
            ),
            (
                Ty::result(Ty::Tuple(vec![Ty::int()]), Ty::string()),
                Ty::Tuple(vec![Ty::int(), Ty::int()]),
                None,
            ),
            (
                Ty::result(Ty::struct_ty("Point", vec![]), Ty::string()),
                con("Point"),
                Some("wrap in Ok(...)"),
            ),
            (
                Ty::int(),
                Ty::float(),
                Some("convert it with `Float.to_int(...)`"),
            ),
            (
                Ty::float(),
                Ty::int(),
                Some("convert it with `Int.to_float(...)`"),
            ),
            (Ty::string(), Ty::int(), Some("use to_string()")),
            (Ty::string(), Ty::float(), Some("use to_string()")),
            (Ty::bool(), Ty::int(), Some("expected a boolean expression")),
            (Ty::string(), Ty::bool(), None),
            (
                Ty::string(),
                Ty::json(),
                Some("encode it: `Json.encode(...)` is its JSON text"),
            ),
            (
                Ty::json(),
                Ty::string(),
                Some("parse it: `Json.parse(...)` reads JSON text into a Json"),
            ),
        ];
        for (expected, found, suggestion) in cases {
            assert_eq!(
                fix_suggestion(&expected, &found),
                suggestion,
                "{expected} / {found}"
            );
        }
    }

    /// A type that lacks a trait is told how to get it: JSON by what JSON
    /// holds, a named type by a derive or an impl, and a type with
    /// parameters that it can have no impl.
    #[test]
    fn missing_traits_are_explained_by_the_type() {
        let file = SourceFile::new("m.mpl", "x");
        let help = |ty: Ty, trait_name: &str| {
            let builtin = ty.con_name() == Some("Regex");
            let error = TypeError::TraitNotSatisfied {
                ty,
                trait_name: trait_name.to_string(),
                builtin,
                origin: ConstraintOrigin::Builtin,
            };
            error.to_diagnostic(&file).repair().to_string()
        };
        let point = || Ty::struct_ty("Point", vec![]);
        assert!(help(Ty::int(), "Json").starts_with("JSON holds Int, Float"));
        assert!(help(point(), "Eq").starts_with("add `deriving(Eq)`"));
        // An Option or a tuple has it by what it holds.
        assert!(help(Ty::option(Ty::int()), "Eq").starts_with("`Option` has `Eq` only when"));
        assert!(help(Ty::Tuple(vec![Ty::int()]), "Display").starts_with("a tuple has `Display`"));
        // A built-in type has no definition to derive on.
        assert_eq!(
            help(Ty::Con(crate::ty::TyCon::new("Regex")), "Display"),
            "add `impl Display for Regex do ... end`"
        );
        assert_eq!(
            help(point(), "Named"),
            "add `impl Named for Point do ... end`"
        );
        assert_eq!(
            help(Ty::list(Ty::int()), "Named"),
            "only a named type without type parameters can have an `impl`"
        );
    }
}
