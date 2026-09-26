//! Every type error as `meshc` shows it, in the terminal and as JSON: one
//! of each kind (and a mismatch from each origin), so a change to how
//! errors render is a snapshot change to review.

use mesh_typeck::diagnostics::{render_diagnostic, DiagnosticOptions};
use mesh_typeck::error::{ConstraintOrigin, TypeError};
use mesh_typeck::ty::{Ty, TyVar};
use rowan::TextRange;

const SOURCE: &str = "fn main() do\n  let value = compute(1, 2)\n  value + other\nend\n";

fn span(start: u32, end: u32) -> TextRange {
    TextRange::new(start.into(), end.into())
}

fn one_of_each() -> Vec<TypeError> {
    vec![
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::Expr { span: span(27, 40) },
        },
        TypeError::InfiniteType {
            var: TyVar(7),
            ty: Ty::int(),
            origin: ConstraintOrigin::Expr { span: span(27, 40) },
        },
        TypeError::ArityMismatch {
            expected: 2,
            found: 3,
            origin: ConstraintOrigin::Expr { span: span(27, 40) },
        },
        TypeError::SlotPipeOutOfRange {
            slot: 4,
            fn_name: "fn_name".to_string(),
            arity: 2,
            span: span(19, 24),
        },
        TypeError::UnboundVariable {
            name: "name".to_string(),
            span: span(19, 24),
            suggestion: Some("suggestion".to_string()),
        },
        TypeError::NotAFunction {
            ty: Ty::int(),
            span: span(19, 24),
        },
        TypeError::TraitNotSatisfied {
            ty: Ty::int(),
            trait_name: "trait_name".to_string(),
            origin: ConstraintOrigin::Expr { span: span(27, 40) },
        },
        TypeError::MissingTraitMethod {
            trait_name: "trait_name".to_string(),
            method_name: "method_name".to_string(),
            impl_ty: "impl_ty".to_string(),
            span: Some(span(27, 40)),
        },
        TypeError::TraitMethodSignatureMismatch {
            trait_name: "trait_name".to_string(),
            method_name: "method_name".to_string(),
            expected: Ty::int(),
            found: Ty::string(),
            span: Some(span(27, 40)),
        },
        TypeError::MissingField {
            struct_name: "struct_name".to_string(),
            field_name: "field_name".to_string(),
            span: span(19, 24),
        },
        TypeError::UnknownField {
            struct_name: "struct_name".to_string(),
            field_name: "field_name".to_string(),
            span: span(19, 24),
        },
        TypeError::NoSuchField {
            ty: Ty::int(),
            field_name: "field_name".to_string(),
            span: span(19, 24),
        },
        TypeError::NoSuchMethod {
            ty: Ty::int(),
            method_name: "method_name".to_string(),
            span: span(19, 24),
        },
        TypeError::ManualContinuityPromotionDisabled { span: span(19, 24) },
        TypeError::UnknownVariant {
            name: "name".to_string(),
            span: span(19, 24),
            suggestion: Some("suggestion".to_string()),
        },
        TypeError::OrPatternBindingMismatch {
            expected_bindings: vec![
                "expected_bindings_a".to_string(),
                "expected_bindings_b".to_string(),
            ],
            found_bindings: vec![
                "found_bindings_a".to_string(),
                "found_bindings_b".to_string(),
            ],
            span: span(19, 24),
        },
        TypeError::NonExhaustiveMatch {
            scrutinee_type: "scrutinee_type".to_string(),
            missing_patterns: vec![
                "missing_patterns_a".to_string(),
                "missing_patterns_b".to_string(),
            ],
            span: span(19, 24),
        },
        TypeError::NonExhaustiveClauses {
            scrutinee_type: "scrutinee_type".to_string(),
            missing_patterns: vec![
                "missing_patterns_a".to_string(),
                "missing_patterns_b".to_string(),
            ],
            span: span(19, 24),
        },
        TypeError::RedundantArm {
            arm_index: 2,
            span: span(19, 24),
        },
        TypeError::SendTypeMismatch {
            expected: Ty::int(),
            found: Ty::string(),
            span: span(19, 24),
        },
        TypeError::SelfOutsideActor { span: span(19, 24) },
        TypeError::SpawnNonFunction {
            found: Ty::string(),
            span: span(19, 24),
        },
        TypeError::ReceiveOutsideActor { span: span(19, 24) },
        TypeError::InvalidChildStart {
            child_name: "child_name".to_string(),
            found: Ty::string(),
            span: span(19, 24),
        },
        TypeError::InvalidStrategy {
            found: "found".to_string(),
            span: span(19, 24),
        },
        TypeError::InvalidRestartType {
            found: "found".to_string(),
            child_name: "child_name".to_string(),
            span: span(19, 24),
        },
        TypeError::InvalidShutdownValue {
            found: "found".to_string(),
            child_name: "child_name".to_string(),
            span: span(19, 24),
        },
        TypeError::CatchAllNotLast {
            fn_name: "fn_name".to_string(),
            arity: 2,
            span: span(19, 24),
        },
        TypeError::NonConsecutiveClauses {
            fn_name: "fn_name".to_string(),
            arity: 2,
            first_span: span(19, 24),
            second_span: span(51, 56),
        },
        TypeError::ClauseArityMismatch {
            fn_name: "fn_name".to_string(),
            expected_arity: 2,
            found_arity: 2,
            span: span(19, 24),
        },
        TypeError::NonFirstClauseAnnotation {
            fn_name: "fn_name".to_string(),
            what: "what".to_string(),
            span: span(19, 24),
        },
        TypeError::DuplicateImpl {
            trait_name: "trait_name".to_string(),
            impl_type: "impl_type".to_string(),
            first_impl: "first_impl".to_string(),
        },
        TypeError::AmbiguousMethod {
            method_name: "method_name".to_string(),
            candidate_traits: vec![
                "candidate_traits_a".to_string(),
                "candidate_traits_b".to_string(),
            ],
            ty: Ty::int(),
            span: span(19, 24),
        },
        TypeError::UnsupportedDerive {
            trait_name: "trait_name".to_string(),
            type_name: "type_name".to_string(),
            span: span(19, 24),
        },
        TypeError::MissingDerivePrerequisite {
            trait_name: "trait_name".to_string(),
            requires: "requires".to_string(),
            type_name: "type_name".to_string(),
            span: span(19, 24),
        },
        TypeError::BreakOutsideLoop { span: span(19, 24) },
        TypeError::ContinueOutsideLoop { span: span(19, 24) },
        TypeError::ImportModuleNotFound {
            module_name: "module_name".to_string(),
            span: span(19, 24),
            suggestion: Some("suggestion".to_string()),
        },
        TypeError::ImportNameNotFound {
            module_name: "module_name".to_string(),
            name: "name".to_string(),
            span: span(19, 24),
            available: vec!["available_a".to_string(), "available_b".to_string()],
        },
        TypeError::PrivateItem {
            module_name: "module_name".to_string(),
            name: "name".to_string(),
            span: span(19, 24),
        },
        TypeError::HttpClusteredInvalidArguments {
            reason: "reason".to_string(),
            span: span(19, 24),
        },
        TypeError::HttpClusteredPrivateHandler {
            handler_name: "handler_name".to_string(),
            span: span(19, 24),
        },
        TypeError::HttpClusteredOutsideRouteHandlerPosition { span: span(19, 24) },
        TypeError::HttpClusteredConflictingReplicationCount {
            runtime_name: "runtime_name".to_string(),
            first_count: 4,
            current_count: 4,
            first_span: span(19, 24),
            span: span(19, 24),
        },
        TypeError::HttpClusteredImportedOriginMissing {
            handler_name: "handler_name".to_string(),
            span: span(19, 24),
        },
        TypeError::TryIncompatibleReturn {
            operand_ty: Ty::int(),
            fn_return_ty: Ty::int(),
            span: span(19, 24),
        },
        TypeError::TryOnNonResultOption {
            operand_ty: Ty::int(),
            span: span(19, 24),
        },
        TypeError::NonSerializableField {
            struct_name: "struct_name".to_string(),
            field_name: "field_name".to_string(),
            field_type: "field_type".to_string(),
            span: span(19, 24),
        },
        TypeError::NonMappableField {
            struct_name: "struct_name".to_string(),
            field_name: "field_name".to_string(),
            field_type: "field_type".to_string(),
            span: span(19, 24),
        },
        TypeError::MissingAssocType {
            trait_name: "trait_name".to_string(),
            assoc_name: "assoc_name".to_string(),
            impl_ty: "impl_ty".to_string(),
        },
        TypeError::ExtraAssocType {
            trait_name: "trait_name".to_string(),
            assoc_name: "assoc_name".to_string(),
            impl_ty: "impl_ty".to_string(),
        },
        TypeError::UnresolvedAssocType {
            assoc_name: "assoc_name".to_string(),
            span: span(19, 24),
        },
        TypeError::UndefinedType {
            alias_name: "alias_name".to_string(),
            target_name: "target_name".to_string(),
            span: span(19, 24),
        },
        TypeError::NativeDeclarationInvalid {
            reason: "reason".to_string(),
            span: span(19, 24),
        },
        TypeError::ExportDeclarationInvalid {
            reason: "reason".to_string(),
            span: span(19, 24),
        },
        TypeError::InvalidLetPattern {
            reason: "reason".to_string(),
            span: span(19, 24),
        },
        TypeError::InvalidPassThroughArm {
            reason: "reason".to_string(),
            span: span(19, 24),
        },
        TypeError::DuplicateBinding {
            name: "name".to_string(),
            span: span(19, 24),
        },
        TypeError::UnboundedTypeParam {
            param: "param".to_string(),
            trait_name: "trait_name".to_string(),
            origin: ConstraintOrigin::Expr { span: span(27, 40) },
        },
        TypeError::AmbiguousImplMethod {
            method: "method".to_string(),
            receiver: Ty::int(),
            candidates: vec![Ty::int(), Ty::string()],
            found: Some(Ty::int()),
            span: span(19, 24),
            by_argument: false,
        },
        TypeError::AmbiguousImplMethod {
            method: "from".to_string(),
            receiver: Ty::Con(mesh_typeck::ty::TyCon::new("Meters")),
            candidates: vec![Ty::int(), Ty::string()],
            found: Some(Ty::bool()),
            span: span(19, 24),
            by_argument: true,
        },
        TypeError::DuplicateDefinition {
            kind: "kind",
            name: "name".to_string(),
            span: span(19, 24),
        },
        TypeError::UnknownType {
            name: "name".to_string(),
            span: span(19, 24),
        },
        TypeError::UnknownFieldOwner {
            field: "field".to_string(),
            span: span(19, 24),
        },
        TypeError::UntypedMethodParam {
            method: "method".to_string(),
            param: "param".to_string(),
            span: span(19, 24),
        },
        TypeError::TypeNotValue {
            name: "Name".to_string(),
            span: span(19, 24),
        },
        TypeError::UnknownInterface {
            name: "name".to_string(),
            span: span(19, 24),
        },
        TypeError::InvalidLiteral {
            reason: "reason".to_string(),
            span: span(19, 24),
        },
        TypeError::NoSuchModuleFunction {
            module: "module".to_string(),
            name: "name".to_string(),
            available: vec!["available_a".to_string(), "available_b".to_string()],
            span: span(19, 24),
        },
        TypeError::AssertReceiveOutsideTest { span: span(19, 24) },
        TypeError::GenericImplTarget {
            name: "name".to_string(),
            span: span(19, 24),
        },
        TypeError::OverloadedFunctionValue {
            name: "name".to_string(),
            arities: vec![1, 3],
            span: span(19, 24),
        },
        TypeError::IndexingUnsupported { span: span(19, 24) },
        TypeError::ActorMessageTypeUnknown {
            actor: "actor".to_string(),
            span: span(19, 24),
        },
        TypeError::TopLevelLet {
            name: "name".to_string(),
            span: span(19, 24),
        },
        TypeError::ModuleNotImported {
            name: "name".to_string(),
            module: "module".to_string(),
            span: span(19, 24),
        },
        TypeError::InvalidConcat {
            op: "op",
            ty: Ty::int(),
            span: span(19, 24),
        },
        TypeError::RigidTypeParam {
            param: "param".to_string(),
            found: Ty::string(),
            span: span(19, 24),
        },
        TypeError::AmbiguousStaticMethod {
            method: "method".to_string(),
            types: vec!["types_a".to_string(), "types_b".to_string()],
            span: span(19, 24),
        },
        TypeError::AmbiguousDefault { span: span(19, 24) },
        TypeError::CyclicAlias {
            alias_name: "alias_name".to_string(),
            span: span(19, 24),
        },
        TypeError::DuplicateVariant {
            variant: "variant".to_string(),
            first_type: "first_type".to_string(),
            second_type: "second_type".to_string(),
            span: span(19, 24),
        },
        TypeError::UnderivableField {
            trait_name: "trait_name".to_string(),
            type_name: "type_name".to_string(),
            field_name: "field_name".to_string(),
            span: span(19, 24),
        },
        TypeError::DuplicateField {
            field_name: "field_name".to_string(),
            span: span(19, 24),
        },
        TypeError::NotAStruct {
            ty: Ty::int(),
            span: span(19, 24),
        },
        TypeError::ResourceViolation {
            reason: "reason".to_string(),
            span: span(19, 24),
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::FnArg {
                call_site: span(27, 40),
                param_idx: 1,
            },
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::BinOp {
                op_span: span(49, 50),
            },
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::IfBranches {
                if_span: span(27, 40),
                then_span: span(19, 24),
                else_span: span(51, 56),
            },
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::Annotation {
                annotation_span: span(19, 24),
            },
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::Return {
                return_span: span(51, 56),
                fn_span: span(0, 12),
            },
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::LetBinding {
                binding_span: span(19, 24),
            },
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::Assignment {
                lhs_span: span(19, 24),
                rhs_span: span(27, 40),
            },
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::Pattern {
                pattern_span: span(19, 24),
            },
        },
        TypeError::Mismatch {
            expected: Ty::int(),
            found: Ty::string(),
            origin: ConstraintOrigin::Builtin,
        },
    ]
}

#[test]
fn every_type_error_renders() {
    let terminal = DiagnosticOptions::colorless();
    let json = DiagnosticOptions {
        json: true,
        ..DiagnosticOptions::colorless()
    };
    let mut rendered = String::new();
    for error in one_of_each() {
        rendered.push_str(&render_diagnostic(
            &error, SOURCE, "main.mpl", &terminal, None,
        ));
        rendered.push_str(&render_diagnostic(&error, SOURCE, "main.mpl", &json, None));
        rendered.push('\n');
    }
    insta::assert_snapshot!(rendered);
}
