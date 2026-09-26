//! Integration tests for Mesh trait system: interface definitions, impl blocks,
//! where clause constraints, compiler-known operator traits, and trait method dispatch.

use mesh_typeck::error::TypeError;
use mesh_typeck::ty::Ty;
use mesh_typeck::{ImportContext, ModuleExports, TypeckResult};

// ── Helpers ────────────────────────────────────────────────────────────

/// Parse Mesh source and run the type checker.
fn check_source(src: &str) -> TypeckResult {
    let parse = mesh_parser::parse(src);
    mesh_typeck::check(&parse)
}

/// Assert that the result has no errors and the final expression type
/// matches the expected type.
fn assert_result_type(result: &TypeckResult, expected: Ty) {
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
    let actual = result
        .result_type
        .as_ref()
        .expect("expected a result type from inference");
    let actual_str = format!("{}", actual);
    let expected_str = format!("{}", expected);
    assert_eq!(
        actual_str, expected_str,
        "expected type `{}`, got `{}`",
        expected_str, actual_str
    );
}

/// Assert that the result contains an error matching the given predicate.
fn assert_has_error<F: Fn(&TypeError) -> bool>(result: &TypeckResult, pred: F, desc: &str) {
    assert!(
        result.errors.iter().any(pred),
        "expected error matching `{}`, got errors: {:?}",
        desc,
        result.errors
    );
}

/// The error for a `Box` shown without a `Display` impl.
fn box_lacks_display(error: &TypeError) -> bool {
    matches!(error, TypeError::TraitNotSatisfied { ty, trait_name, .. }
        if trait_name == "Display" && ty.to_string() == "Box")
}

// ── Interface definition and impl ────────────────────────────────────

/// 1. Parse `interface Printable do fn to_string(self) -> String end`.
///    Check it registers without errors.
#[test]
fn test_interface_definition() {
    let result = check_source("interface Printable do\n  fn to_string(self) -> String\nend");
    assert!(
        result.errors.is_empty(),
        "interface definition should register without errors, got: {:?}",
        result.errors
    );
}

/// 2. Parse interface + impl for Int. Check it type-checks without errors.
#[test]
fn test_impl_block() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend",
    );
    assert!(
        result.errors.is_empty(),
        "impl block should type-check without errors, got: {:?}",
        result.errors
    );
}

/// 3. Impl block missing a required method. Check for error.
#[test]
fn test_impl_missing_method() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\nend",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::MissingTraitMethod { .. }),
        "MissingTraitMethod",
    );
}

/// 4. Impl with wrong return type. Check for mismatch.
#[test]
fn test_impl_wrong_method_signature() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> Int do\n    42\n  end\nend",
    );
    assert_has_error(
        &result,
        |e| {
            matches!(
                e,
                TypeError::TraitMethodSignatureMismatch { .. } | TypeError::Mismatch { .. }
            )
        },
        "TraitMethodSignatureMismatch or Mismatch",
    );
}

// ── Where clauses ────────────────────────────────────────────────────

/// 5. Function with where clause called with satisfying type.
///    Note: Mesh uses `::` for type annotations in params (not `:`).
#[test]
fn test_where_clause_satisfied() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend\n\
         fn show<T>(x :: T) -> String where T: Printable do\n  to_string(x)\nend\n\
         show(42)",
    );
    assert_result_type(&result, Ty::string());
}

/// 6. Call with type lacking required impl.
#[test]
fn test_where_clause_unsatisfied() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         fn show<T>(x :: T) -> String where T: Printable do\n  to_string(x)\nend\n\
         show(true)",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::TraitNotSatisfied { .. }),
        "TraitNotSatisfied",
    );
}

/// 7. Multiple constraints on same type param.
#[test]
fn test_multiple_where_constraints() {
    let result = check_source(
        "interface Printable do\n  fn to_string(self) -> String\nend\n\
         interface Debuggable do\n  fn debug(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend\n\
         impl Debuggable for Int do\n  fn debug(self) -> String do\n    \"dbg:int\"\n  end\nend\n\
         fn show_debug<T>(x :: T) -> String where T: Printable, T: Debuggable do\n  to_string(x)\nend\n\
         show_debug(42)",
    );
    assert_result_type(&result, Ty::string());
}

// ── Compiler-known traits for operators ──────────────────────────────

/// 8. `1 + 2` -> Int (via Add trait for Int).
#[test]
fn test_add_trait_int() {
    let result = check_source("1 + 2");
    assert_result_type(&result, Ty::int());
}

/// 9. `1.0 + 2.0` -> Float (via Add trait for Float).
#[test]
fn test_add_trait_float() {
    let result = check_source("1.0 + 2.0");
    assert_result_type(&result, Ty::float());
}

/// 10. `"a" + "b"` fails (no Add for String).
#[test]
fn test_add_trait_string_fails() {
    let result = check_source("\"a\" + \"b\"");
    assert_has_error(
        &result,
        |e| {
            matches!(
                e,
                TypeError::TraitNotSatisfied { .. } | TypeError::Mismatch { .. }
            )
        },
        "TraitNotSatisfied or Mismatch (no Add for String)",
    );
}

/// 11. `1 == 2` -> Bool (via Eq trait).
#[test]
fn test_eq_trait() {
    let result = check_source("1 == 2");
    assert_result_type(&result, Ty::bool());
}

/// 12. `1 < 2` -> Bool (via Ord trait).
#[test]
fn test_ord_trait() {
    let result = check_source("1 < 2");
    assert_result_type(&result, Ty::bool());
}

// ── Trait method dispatch ────────────────────────────────────────────

/// 13. Call trait method on concrete type with registered impl.
#[test]
fn test_trait_method_call() {
    let interface = "interface Printable do\n  fn to_string(self) -> String\nend\n\
         impl Printable for Int do\n  fn to_string(self) -> String do\n    \"int\"\n  end\nend\n";
    // Display and Printable both give Int a `to_string`: a bare call, like
    // `42.to_string()`, is ambiguous, and naming the interface resolves it.
    let result = check_source(&format!("{interface}to_string(42)"));
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::AmbiguousMethod { .. }),
        "AmbiguousMethod",
    );
    let result = check_source(&format!("{interface}Printable.to_string(42)"));
    assert_result_type(&result, Ty::string());
}

/// In an impl, `-> Self` is the implementing type: it was left as `Self`,
/// so the body (`Size { ... }`) mismatched and callers saw a type `Self`
/// with no fields.
#[test]
fn impl_method_self_is_the_implementing_type() {
    let result = check_source(
        "interface Growable do\n  fn grow(self) -> Self\nend\n\nstruct Size do\n  n :: Int\nend\n\nimpl Growable for Size do\n  fn grow(self) -> Self do\n    Size { n: self.n + 1 }\n  end\nend\n\nfn main() do\n  let s = Size { n: 1 }\n  s.grow().n\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// `Iter<T>` is the pipeline type `Iter.*` returns; naming it in an
/// annotation was E0069 "unknown type".
#[test]
fn iter_is_a_known_type_in_annotations() {
    let result = check_source(
        "fn evens(xs :: List<Int>) -> Iter<Int> do\n  Iter.filter(Iter.from(xs), fn x -> x % 2 == 0 end)\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// `describe`, `test`, `setup` and `teardown` are test-DSL builtins in every
/// file; a bare call of an interface method with one of those names reached
/// the builtin ("expected 2 argument(s), found 1").
#[test]
fn interface_methods_may_share_a_test_dsl_name() {
    let result = check_source(
        "interface Describe do\n  fn describe(self) -> String\nend\n\nstruct P do\n  x :: Int\nend\n\nimpl Describe for P do\n  fn describe(self) -> String do\n    \"p\"\n  end\nend\n\nfn main() do\n  let p = P { x: 1 }\n  describe(p)\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// What an interpolation or operator needs of its operand is checked in an
/// actor's body and in a multi-clause function as in any other function:
/// it went unchecked there, and code generation failed with no location.
#[test]
fn operand_traits_are_checked_outside_single_clause_functions() {
    for src in [
        "struct Box do\n  n :: Int\nend\n\nactor shower() do\n  receive do\n    n -> println(\"#{Box { n: n }}\")\n  end\nend\n\nfn main() do\n  let pid = spawn(shower)\n  send(pid, 1)\nend\n",
        "struct Box do\n  n :: Int\nend\n\nfn show(0) = \"zero\"\nfn show(n) = \"#{Box { n: n }}\"\n\nfn main() do\n  println(show(1))\nend\n",
    ] {
        let result = check_source(src);
        assert_has_error(
            &result,
            box_lacks_display,
            src,
        );
    }
}

/// What a function's body needs of a parameter it leaves generic is
/// required of the argument however the call is written: piped (`|>`,
/// `|N>`, a bare `|> f`) as well as direct, and of a function written
/// `fn f(x) = ...` as of one with a `do` body. Pipes and the `=` form let a
/// `Box` with no `Display` through to code generation.
#[test]
fn a_parameters_inferred_bounds_hold_for_every_call_form() {
    let prelude = "struct Box do\n  n :: Int\nend\n\nfn show_block(x) do\n  \"#{x}\"\nend\n\nfn show_eq(x) = \"#{x}\"\n\nfn pair(a, x) do\n  \"#{a} #{x}\"\nend\n\n";
    for call in [
        "show_block(b)",
        "b |> show_block()",
        "b |> show_block",
        "show_eq(b)",
        "b |> show_eq",
        "1 |2> pair(b)",
        "b |> String.from()",
    ] {
        let src =
            format!("{prelude}fn main() do\n  let b = Box {{ n: 1 }}\n  println({call})\nend\n");
        let result = check_source(&src);
        assert_has_error(&result, box_lacks_display, call);
    }
    let fine = format!(
        "{prelude}fn main() do\n  println(1 |> show_eq())\n  println(2 |2> pair(\"a\"))\nend\n"
    );
    assert!(check_source(&fine).errors.is_empty());
}

/// An imported function requires of its callers' arguments what it would
/// of calls in its own module: its where-clause and the bounds its body
/// infers. Neither crossed the module boundary, and a `Box` with no
/// `Display` was interpolated anyway.
#[test]
fn an_imported_functions_requirements_hold_for_its_callers() {
    let util = mesh_parser::parse(
        "pub fn show(x) do\n  \"#{x}\"\nend\n\npub fn show_eq(x) = \"#{x}\"\n\npub fn show_where<T>(x :: T) -> String where T: Display do\n  \"#{x}\"\nend\n",
    );
    let exports = mesh_typeck::collect_exports(&util, &mesh_typeck::check(&util));
    let mut imports = ImportContext::empty();
    imports
        .module_exports
        .insert("Util".into(), ModuleExports::new("Util".into(), &exports));
    let check = |body: &str| {
        let src = format!("import Util\nfrom Util import show\n\nstruct Box do\n  n :: Int\nend\n\nfn main() do\n  let b = Box {{ n: 1 }}\n  {body}\nend\n");
        mesh_typeck::check_with_imports(&mesh_parser::parse(&src), &imports)
    };
    for call in [
        "show(b)",
        "Util.show(b)",
        "b |> Util.show()",
        "Util.show_eq(b)",
        "Util.show_where(b)",
    ] {
        assert_has_error(&check(&format!("println({call})")), box_lacks_display, call);
    }
    let fine = check("println(show(1))\n  println(Util.show_where(b.n))");
    assert!(fine.errors.is_empty(), "{:?}", fine.errors);
}

/// A named function passed as a value requires of whatever its parameters
/// become what a call of it would: its where-clause and the bounds its body
/// infers. Passed to `apply` or `List.map`, or bound with `let` and
/// called, `show_block` interpolated a `Box` with no `Display`, or code
/// generation failed with no location.
/// A parameter that shadows the function is a value of its own.
#[test]
fn a_function_used_as_a_value_keeps_its_requirements() {
    let prelude = "struct Box do\n  n :: Int\nend\n\nfn show_block(x) do\n  \"#{x}\"\nend\n\nfn show_where<T>(x :: T) -> String where T: Display do\n  \"shown\"\nend\n\nfn apply(f, x) do\n  f(x)\nend\n\n";
    for body in [
        "apply(show_block, b)",
        "apply(show_where, b)",
        "List.map([b], show_block)",
        "show_where(b)",
        "let f = show_block\n  f(b)",
        "let f = show_block\n  let g = f\n  g(b)",
    ] {
        let src = format!("{prelude}fn main() do\n  let b = Box {{ n: 1 }}\n  {body}\nend\n");
        assert_has_error(&check_source(&src), box_lacks_display, body);
    }
    let shadowed = format!("{prelude}fn g(show_block, b) do\n  show_block(b)\nend\n\nfn main() do\n  g(fn(x) -> x.n end, Box {{ n: 1 }})\nend\n");
    let result = check_source(&shadowed);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// A where-clause holds for a function written `fn f<T>(x :: T) = ...`:
/// its parameters' annotations were ignored, so no argument was checked
/// against the clause.
#[test]
fn a_where_clause_holds_for_an_expression_bodied_function() {
    let src = "struct Box do\n  n :: Int\nend\n\nfn show<T>(x :: T) -> String where T: Display = \"shown\"\n\nfn main() do\n  show(Box { n: 1 })\nend\n";
    assert_has_error(&check_source(src), box_lacks_display, "show(Box)");
}

/// A method is compiled once, for the type it is implemented for, so a
/// parameter whose type nothing fixes cannot be compiled: `label(self, x)`
/// read a `Box` argument as `()`, and an `Int` one failed in LLVM. A type
/// the body fixes, or an annotation, is enough.
#[test]
fn a_method_parameter_needs_a_type() {
    let prelude = "struct Wrap do\n  tag :: String\nend\n\ninterface Labeler do\n  fn label(self, x) -> String\nend\n\n";
    let open = format!("{prelude}impl Labeler for Wrap do\n  fn label(self, x) -> String do\n    \"#{{self.tag}}: #{{x}}\"\n  end\nend\n\nfn main() do\n  println(Wrap {{ tag: \"t\" }}.label(5))\nend\n");
    let result = check_source(&open);
    assert!(
        result.errors.iter().any(|error| matches!(
            error,
            TypeError::UntypedMethodParam { method, param, .. } if method == "label" && param == "x"
        )),
        "{:?}",
        result.errors
    );
    for body in [
        "x :: Int) -> String do\n    \"#{self.tag}: #{x}\"",
        "x) -> String do\n    \"#{self.tag}: #{x + 1}\"",
    ] {
        let typed = format!("{prelude}impl Labeler for Wrap do\n  fn label(self, {body}\n  end\nend\n\nfn main() do\n  println(Wrap {{ tag: \"t\" }}.label(5))\nend\n");
        let result = check_source(&typed);
        assert!(result.errors.is_empty(), "{body}: {:?}", result.errors);
    }
}

/// A struct's name is no value: `let x = Wrap` and `Wrap.tag` type checked
/// as a `Wrap` value and failed in code generation, and `Wrap.label(5)` in
/// LLVM. An instance method named on the type takes the value first, as
/// one named on its interface does: `Wrap.label(w, 5)` said "expected 2
/// argument(s), found 3".
#[test]
fn a_type_is_not_a_value_but_names_its_methods() {
    let prelude = "struct Wrap do\n  tag :: String\nend\n\ninterface Labeler do\n  fn label(self, x :: Int) -> String\nend\n\nimpl Labeler for Wrap do\n  fn label(self, x :: Int) -> String do\n    \"#{self.tag}: #{x}\"\n  end\nend\n\n";
    let check = |body: &str| {
        check_source(&format!(
            "{prelude}fn main() do\n  let w = Wrap {{ tag: \"t\" }}\n  {body}\nend\n"
        ))
    };
    let not_value = check("let x = Wrap\n  x");
    assert!(
        not_value.errors.iter().any(|error| matches!(
            error,
            TypeError::TypeNotValue { name, .. } if name == "Wrap"
        )),
        "{:?}",
        not_value.errors
    );
    for body in ["Wrap.tag", "Wrap.label(5)"] {
        assert!(!check(body).errors.is_empty(), "{body}");
    }
    for body in [
        "Wrap.label(w, 5)",
        "w |> Wrap.label(5)",
        "w |> Labeler.label(5)",
        "5 |2> Wrap.label(w)",
    ] {
        let result = check(body);
        assert!(result.errors.is_empty(), "{body}: {:?}", result.errors);
    }
}

/// An interface's default method is compiled for each implementing type,
/// so it needs its parameters' types as an impl's method does: `times`
/// failed LLVM verification ("Call parameter type does not match").
#[test]
fn a_default_method_parameter_needs_a_type() {
    let src = "interface Greeter do\n  fn name(self) -> String\n  fn greet(self, greeting :: String, times) -> String do\n    \"#{greeting} #{self.name()} x#{times}\"\n  end\nend\n\nfn main() do\n  nil\nend\n";
    let result = check_source(src);
    assert!(
        result.errors.iter().any(|error| matches!(
            error,
            TypeError::UntypedMethodParam { method, param, .. } if method == "greet" && param == "times"
        )),
        "{:?}",
        result.errors
    );
    let typed = check_source(&src.replace("times)", "times :: Int)"));
    assert!(typed.errors.is_empty(), "{:?}", typed.errors);
}

/// An operator needs its trait of a known operand type: `<` Ord, `+` Add,
/// unary `-` Neg. A struct deriving nothing has none of them, and `Bool`
/// cannot be added or negated.
#[test]
fn operators_need_their_traits_of_known_types() {
    let prelude = "struct Box do\n  n :: Int\nend deriving()\n\n";
    for (body, trait_name) in [
        ("Box { n: 1 } < Box { n: 2 }", "Ord"),
        ("true + false", "Add"),
        ("-true", "Neg"),
        ("-Box { n: 1 }", "Neg"),
    ] {
        let result = check_source(&format!("{prelude}fn main() do\n  {body}\nend\n"));
        assert_has_error(
            &result,
            |error| matches!(error, TypeError::TraitNotSatisfied { trait_name: found, .. } if found == trait_name),
            body,
        );
    }
}
