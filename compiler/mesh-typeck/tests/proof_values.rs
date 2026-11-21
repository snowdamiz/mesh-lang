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
    assert!(checked
        .errors
        .iter()
        .any(|error| error.to_string().contains("requires an embedded proof")));
    assert!(!collect_exports(&parse, &checked)
        .functions
        .contains_key("allowed_two"));
}
