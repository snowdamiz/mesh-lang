//! The generated test observer consumes a returned owner; generic calls do not.
use mesh_typeck::{error::TypeError, ImportContext};
fn check(source: &str, test: bool) -> mesh_typeck::TypeckResult {
    let p = mesh_parser::parse(source);
    assert!(p.errors().is_empty(), "{:?}", p.errors());
    let mut c = ImportContext::empty();
    c.test_builtins = test;
    mesh_typeck::check_with_imports(&p, &c)
}
#[test]
fn generated_test_outcome_consumes_whole_result_and_each_variant() {
    let source="fn __test_body_check(value :: Result<SecretBytes, SecretBytes>) do __test_check_result(value) end";
    assert!(check(source, true).errors.is_empty());
    let negative="fn __test_body_check(value :: Result<SecretBytes, SecretBytes>) do __test_check_result(value); value end";
    let t = check(negative, true);
    assert!(
        t.errors
            .iter()
            .any(|e| matches!(e, TypeError::ResourceViolation { .. })),
        "{:?}",
        t.errors
    );
    assert!(
        t.errors
            .iter()
            .all(|e| matches!(e, TypeError::ResourceViolation { .. })),
        "{:?}",
        t.errors
    );
}
#[test]
fn test_outcome_transfer_rule_is_not_generic_or_shadow_authority() {
    for (source,test) in [
 ("fn __test_body_check(value :: Result<SecretBytes, SecretBytes>) do let indirect=__test_check_result; indirect(value) end",true),
 ("fn __test_check_result(value) do nil end\nfn __test_body_check(value :: Result<SecretBytes, SecretBytes>) do __test_check_result(value) end",true),
 ] {let t=check(source,test);assert!(t.errors.iter().any(|e|matches!(e,TypeError::ResourceViolation{..})),"{source}: {:?}",t.errors);assert!(t.errors.iter().all(|e|matches!(e,TypeError::ResourceViolation{..})),"{:?}",t.errors);}
}

#[test]
fn internal_test_observer_is_not_registered_in_ordinary_production() {
    for value in ["Ok(nil)", "nil"] {
        let source = format!("fn ordinary() do __test_check_result({value}) end");
        let t = check(&source, false);
        assert!(
            t.errors.iter().any(
                |e| matches!(e,TypeError::UnboundVariable{name,..} if name=="__test_check_result")
            ),
            "{:?}",
            t.errors
        );
    }
}
