use mesh_typeck::{
    check, check_with_imports, collect_exports, error::TypeError, ImportContext, ModuleExports,
};

const VECTOR: &str = include_str!("../../meshc/tests/fixtures/proofs/evidence_native/vector.mpl");
const MAIN: &str = include_str!("../../meshc/tests/fixtures/proofs/evidence_native/main.mpl");

fn vector_import() -> ImportContext {
    let parse = mesh_parser::parse(VECTOR);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let checked = check(&parse);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let exports = collect_exports(&parse, &checked);
    let witness = &exports.functions["allowed_two"];
    assert!(witness.vars.is_empty());
    assert!(witness.ty.is_proof());
    assert_eq!(witness.ty.to_string(), "Proof<FitsTwo<2>>");
    assert!(exports.functions["deliver"].ty.contains_proof());
    assert!(!exports.functions.contains_key("fits_when_equal"));
    let mut context = ImportContext::empty();
    context.module_exports.insert(
        "Vector".into(),
        ModuleExports::new("Vector".into(), &exports),
    );
    context
}

#[test]
fn imports_closed_theorem_as_a_value_and_matches_the_lexical_index() {
    let context = vector_import();
    let parse = mesh_parser::parse(MAIN);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let checked = check_with_imports(&parse, &context);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    assert!(checked.types.values().any(|ty| ty.is_proof()));

    let source = format!("{MAIN}\nfn forward(witness :: Proof<FitsTwo<Succ<1>>>) do\n  witness\nend\nfn forward_two() do\n  forward(allowed_two)\nend");
    let checked = check_with_imports(&mesh_parser::parse(&source), &context);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
}

#[test]
fn rejects_evidence_for_a_different_index_and_rejects_unit() {
    let context = vector_import();
    for (source, expected, found) in [
        (MAIN.replace("deliver(double,", "deliver(single,"), "1", "2"),
        (
            MAIN.replace("deliver(double, allowed_two)", "deliver(double, ())"),
            "Proof<FitsTwo<2>>",
            "()",
        ),
    ] {
        let checked = check_with_imports(&mesh_parser::parse(&source), &context);
        assert!(
            checked.errors.iter().any(|error| {
                matches!(error, TypeError::Mismatch { expected: left, found: right, .. }
                if left.to_string() == expected && right.to_string() == found)
            }),
            "{:?}",
            checked.errors
        );
    }
}

#[test]
fn rejects_non_logic_values_and_foreign_proof_results() {
    let context = vector_import();
    for source in [
        "from Vector import allowed_two\nstruct Proof<P> do end",
        "from Vector import allowed_two\n@native(\"foreign_witness\")\npub fn foreign_witness() -> Proof<FitsTwo<2>>",
        "from Vector import allowed_two\nfn f(x :: Proof<FitsTwo<Int>>) do x end",
    ] {
        let parse = mesh_parser::parse(source);
        assert!(parse.errors().is_empty(), "{:?}", parse.errors());
        let checked = check_with_imports(&parse, &context);
        assert!(!checked.errors.is_empty(), "{source}");
    }
}

#[test]
fn public_theorem_must_have_a_matching_embedded_proof() {
    let source = VECTOR.replace(
        "  proof allowed_two do\n    exact fits_when_equal(2, Eq.refl(2))\n  end\n",
        "",
    );
    let parse = mesh_parser::parse(&source);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let checked = check(&parse);
    assert!(
        checked.errors.iter().any(|error| matches!(
            error,
            TypeError::MissingEmbeddedProof { theorem, .. } if theorem == "allowed_two"
        )),
        "{:?}",
        checked.errors
    );
    assert!(!collect_exports(&parse, &checked)
        .functions
        .contains_key("allowed_two"));
}

const VEC: &str = "type Vec<T, n :: Nat> do
  Empty -> Vec<T, 0>
  Push<k :: Nat>(T, Vec<T, k>) -> Vec<T, Succ<k>>
end
";

const ADD_ZERO: &str = "logic do
  pub theorem add_zero(n :: Nat) do n + 0 == n end
  proof add_zero do refl end
end
";

/// Each proof and index error has the code of its repair, where all of
/// them were E0095 "invalid Nat index".
#[test]
fn each_proof_and_index_error_has_its_own_code() {
    let deep_logic = format!(
        "logic do\n  theorem t() do {}0{} end\nend\n",
        "(".repeat(70),
        ")".repeat(70)
    );
    for (source, code) in [
        ("fn count<n :: Int>() -> Int = 0".to_string(), "E0095"),
        (
            "fn bad<x>(h :: Proof<NatEq<NatMul<0, x>, 0>>) do h end".to_string(),
            "E0095",
        ),
        (
            format!("{VEC}fn bad(v :: Vec<Int, Int>) do v end"),
            "E0095",
        ),
        (
            VEC.replace("Empty -> Vec<T, 0>", "Empty"),
            "E0096",
        ),
        (format!("{VEC}fn bad(v :: Vec) do v end"), "E0096"),
        (
            format!("{VEC}type Wrong<T, n :: Nat> do\n  Wrong -> Vec<T, 0>\nend\n"),
            "E0096",
        ),
        ("struct Proof do\n  value :: Int\nend".to_string(), "E0097"),
        (
            "logic do\n  pub theorem two() do 2 == 2 end\nend".to_string(),
            "E0098",
        ),
        (
            "logic do\n  pub theorem two() do 2 == 2 end\n  proof two do refl end\n  pub theorem two() do 1 + 1 == 2 end\nend".to_string(),
            "E0099",
        ),
        (
            "fn accept(evidence :: Proof<NatEq<2>>) -> Int = 2".to_string(),
            "E0100",
        ),
        (
            "fn accept(evidence :: Proof<Ready>) -> Int = 2".to_string(),
            "E0100",
        ),
        (
            "logic do\n  pub theorem yes() do true end\n  proof yes do refl end\nend".to_string(),
            "E0100",
        ),
        (
            format!("{ADD_ZERO}fn main() do\n  let evidence = add_zero\nend"),
            "E0100",
        ),
        (
            "logic do\n  theorem two() do 2 = 2 end\nend".to_string(),
            "P0301",
        ),
        (deep_logic, "P0302"),
    ] {
        let parse = mesh_parser::parse(&source);
        assert!(parse.errors().is_empty(), "{source}: {:?}", parse.errors());
        let checked = check(&parse);
        let codes: Vec<_> = checked.errors.iter().map(|e| e.code().id()).collect();
        assert_eq!(codes, [code], "{source}: {:?}", checked.errors);
    }
}

#[test]
fn a_logic_syntax_error_is_a_parse_error_not_an_invalid_index() {
    let source = "fn main() do\nend\nlogic do\n  theorem two() do 2 = 2 end\nend\n";
    let checked = check(&mesh_parser::parse(source));
    let [TypeError::LogicSyntax { error }] = &checked.errors[..] else {
        panic!("{:?}", checked.errors);
    };
    let file = mesh_diagnostics::SourceFile::new("main.mpl", source);
    let diagnostic = checked.errors[0].to_diagnostic(&file);
    assert_eq!(diagnostic.code().id(), "P0301");
    assert_eq!(diagnostic.message(), "expected `end`, found `=`");
    assert_eq!(diagnostic.primary_label(), Some("expected `end` here"));
    assert_eq!(diagnostic.span(), Some(error.span));
    assert_eq!(
        &source[error.span.start as usize..error.span.end as usize],
        "="
    );
    let [opened] = diagnostic.labels() else {
        panic!("{diagnostic:?}");
    };
    assert_eq!(opened.message(), "block opened here");
    assert_eq!(
        &source[opened.span().start as usize..opened.span().end as usize],
        "do"
    );
    assert_ne!(
        diagnostic.repair(),
        mesh_diagnostics::Code::InvalidNatIndex.info().repair
    );
}
