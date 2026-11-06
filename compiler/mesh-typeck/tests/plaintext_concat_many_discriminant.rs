//! Bounded-join forwarding and tag review preserve every unreviewed dependency.
use mesh_typeck::error::TypeError;
use mesh_typeck::plaintext::{Dependencies, Projection};
use mesh_typeck::{ImportContext, ModuleExports, TypeckResult};

fn checked(source: &str) -> TypeckResult {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{source}\n{:?}", parsed.errors());
    mesh_typeck::check(&parsed)
}

fn allowed(source: &str) {
    let result = checked(source);
    assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
}

fn refused_result(source: &str, result: &TypeckResult) {
    assert!(!result.errors.is_empty(), "escaped: {source}");
    assert!(
        result
            .errors
            .iter()
            .all(|error| matches!(error, TypeError::PlaintextViolation { .. })),
        "unrelated error: {source}\n{:?}",
        result.errors
    );
}

fn refused(source: &str) {
    refused_result(source, &checked(source));
}

#[test]
fn helpers_and_function_values_cannot_wash_join_dependencies() {
    for operation in ["Bytes.concat_many", "bytes_concat_many"] {
        let helper = format!(
            "fn join(parts :: List<Bytes>, limit :: Int) -> Bytes!BytesError do\nlet operation = {operation}\noperation(parts,limit) end\n"
        );
        allowed(&format!(
            "{helper}fn public() do let forwarded = join\nlet joined = Result.unwrap_or(forwarded([Bytes.from_utf8(\"public\")],6),Bytes.empty())\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),joined) end"
        ));
        for tail in [
            "println(String.from(Result.is_ok(result)))",
            "println(Int.to_string(Bytes.length(Result.unwrap_or(result,Bytes.empty()))))",
            "Http.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(result,Bytes.empty()))",
        ] {
            refused(&format!(
                "{helper}fn leak(value :: Plaintext<Bytes>) do let forwarded = join\nlet result = forwarded([value],100)\n{tail} end"
            ));
        }
    }
}

#[test]
fn private_list_count_and_list_selection_survive_join_and_tag_review() {
    for operation in ["Bytes.concat_many", "bytes_concat_many"] {
        refused(&format!(
            "fn leak(count :: Plaintext<Int>) do\nlet parts = List.take([Bytes.from_utf8(\"a\"),Bytes.from_utf8(\"bb\")],count)\nlet reviewed = declassify_discriminant({operation}(parts,3),\"test-only join tag\")\nprintln(Int.to_string(Bytes.length(Result.unwrap_or(reviewed,Bytes.empty())))) end"
        ));
        refused(&format!(
            "fn leak(choice :: Plaintext<Bool>) do\nlet parts = if choice do [Bytes.from_utf8(\"a\")] else [Bytes.from_utf8(\"bb\")] end\nprintln(String.from(Result.is_ok({operation}(parts,1)))) end"
        ));
        refused(&format!(
            "fn leak(choice :: Plaintext<Bool>) do\nlet parts = if choice do [Bytes.from_utf8(\"a\")] else [Bytes.from_utf8(\"bb\")] end\nlet reviewed = declassify_discriminant({operation}(parts,2),\"test-only join tag\")\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(reviewed,Bytes.empty())) end"
        ));
    }
}

#[test]
fn reviewed_helper_summary_clears_only_the_tag_not_ok_err_or_length() {
    for operation in ["Bytes.concat_many", "bytes_concat_many"] {
        let source = format!(
            "fn review(parts :: List<Bytes>, limit :: Int) -> Bytes!BytesError do\ndeclassify_discriminant({operation}(parts,limit),\"test-only bounded join status\") end"
        );
        let result = checked(&source);
        assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
        let summary = result.plaintext.functions.get("review").unwrap();
        assert!(summary.effects.is_empty(), "{:?}", summary.effects);
        assert_eq!(
            summary.output_shape.get(&vec![Projection::Tag]),
            Some(&Dependencies::default()),
            "the explicit tag review must be represented in the helper summary"
        );
        // Generic pure calls may use the whole output as the conservative
        // fallback for payload paths. An explicit public override on either
        // payload or on the successful length must still fail this assertion.
        for path in [
            vec![Projection::Variant("Ok".into(), 0)],
            vec![Projection::Variant("Err".into(), 0)],
            vec![Projection::Variant("Ok".into(), 0), Projection::Length],
        ] {
            let dependencies = (1..=path.len())
                .rev()
                .find_map(|length| summary.output_shape.get(&path[..length]))
                .unwrap_or(&summary.output);
            for parameter in [0, 1] {
                assert!(
                    dependencies.parameters.contains(&parameter)
                        || dependencies
                            .projections
                            .iter()
                            .any(|(index, _)| *index == parameter),
                    "{operation}: {path:?} lost parameter {parameter}: {dependencies:?}"
                );
            }
        }
    }
}

#[test]
fn reviewed_helper_permits_status_but_not_payload_fallback_or_caller_control() {
    for operation in ["Bytes.concat_many", "bytes_concat_many"] {
        let helper = format!(
            "fn review(parts :: List<Bytes>, limit :: Int) -> Bytes!BytesError do\ndeclassify_discriminant({operation}(parts,limit),\"test-only bounded join status\") end\n"
        );
        allowed(&format!(
            "{helper}fn status(value :: Plaintext<Bytes>) do\nprintln(String.from(Result.is_ok(review([value],100)))) end"
        ));
        for tail in [
            "Http.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(review([value],100),Bytes.empty()))",
            "println(Int.to_string(Bytes.length(Result.unwrap_or(review([value],100),Bytes.empty()))))",
            "Http.body_bytes(Http.build(:post,\"https://example.test\"),Result.unwrap_or(review([Bytes.empty()],0),value))",
        ] {
            refused(&format!(
                "{helper}fn leak(value :: Plaintext<Bytes>) do {tail} end"
            ));
        }
        refused(&format!(
            "{helper}fn leak(choice :: Plaintext<Bool>) do if choice do\nprintln(String.from(Result.is_ok(review([Bytes.empty()],0)))) end end"
        ));
        refused(&format!(
            "{helper}fn leak(choice :: Plaintext<Bool>) do\nif choice do return nil end\nprintln(String.from(Result.is_ok(review([Bytes.empty()],0)))) end"
        ));
    }
}

#[test]
fn local_lookalikes_use_their_bodies_instead_of_the_builtin_join_rule() {
    for (declaration, operation) in [
        (
            "module Bytes do pub fn concat_many(parts :: List<Bytes>,limit :: Int) -> Bytes!String do BODY end end",
            "Bytes.concat_many",
        ),
        (
            "fn bytes_concat_many(parts :: List<Bytes>,limit :: Int) -> Bytes!String do BODY end",
            "bytes_concat_many",
        ),
    ] {
        let ignores_input = declaration.replace("BODY", "Ok(bytes_from_utf8(\"public\"))");
        allowed(&format!(
            "{ignores_input}\nfn public(value :: Plaintext<Bytes>) do\nprintln(String.from(Result.is_ok({operation}([value],0)))) end"
        ));
        let private_body = declaration.replace(
            "BODY",
            "let output = bytes_from_utf8(\"a\")\nlet choice :: Plaintext<Bool> = true\nif choice do Ok(output) else Err(\"selected\") end",
        );
        // The custom computation is allowed on its own. Its returned private
        // selection, rather than an unrelated sink in its body, must be what
        // makes the caller's public status output fail.
        allowed(&private_body);
        refused(&format!(
            "{private_body}\nfn leak() do\nprintln(String.from(Result.is_ok({operation}([],0)))) end"
        ));
    }
}

#[test]
fn imported_lookalikes_keep_their_own_dependency_summaries() {
    for (body, should_allow) in [
        ("Ok(bytes_from_utf8(\"public\"))", true),
        (
            "let output = bytes_from_utf8(\"a\")\nlet choice :: Plaintext<Bool> = true\nif choice do Ok(output) else Err(\"selected\") end",
            false,
        ),
    ] {
        let dependency = mesh_parser::parse(&format!(
            "pub fn concat_many(parts :: List<Bytes>,limit :: Int) -> Bytes!String do {body} end\npub fn bytes_concat_many(parts :: List<Bytes>,limit :: Int) -> Bytes!String do {body} end"
        ));
        assert!(dependency.ok(), "{:?}", dependency.errors());
        let result = mesh_typeck::check(&dependency);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let exports = mesh_typeck::collect_exports(&dependency, &result);
        let mut imports = ImportContext::empty();
        imports.module_exports.insert(
            "Untrusted.Bytes".into(),
            ModuleExports::new("Untrusted.Bytes".into(), &exports),
        );
        for (import, operation) in [
            ("import Untrusted.Bytes", "Bytes.concat_many"),
            (
                "from Untrusted.Bytes import bytes_concat_many",
                "bytes_concat_many",
            ),
        ] {
            let source = format!(
                "{import}\nfn route(value :: Plaintext<Bytes>) do\nprintln(String.from(Result.is_ok({operation}([value],0)))) end"
            );
            let parsed = mesh_parser::parse(&source);
            assert!(parsed.ok(), "{source}\n{:?}", parsed.errors());
            let result = mesh_typeck::check_with_imports(&parsed, &imports);
            if should_allow {
                assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
            } else {
                refused_result(&source, &result);
            }
            assert!(result.plaintext.sites.is_empty());
        }
    }
}
