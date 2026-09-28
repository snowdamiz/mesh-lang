//! Integration tests for the Mesh type inference engine.
//!
//! These tests parse Mesh source code, run type checking via `mesh_typeck::check()`,
//! and assert on the inferred types and errors. They exercise the core behaviors
//! of Algorithm J inference: literals, let-bindings, let-polymorphism, occurs check,
//! if-branches, function application, closures, arithmetic, and error detection.

use mesh_typeck::error::TypeError;
use mesh_typeck::ty::Ty;
use mesh_typeck::TypeckResult;

// ── Helpers ────────────────────────────────────────────────────────────

/// Parse Mesh source and run the type checker.
fn check_source(src: &str) -> TypeckResult {
    let parse = mesh_parser::parse(src);
    // Uncomment to debug parse failures:
    // if !parse.ok() {
    //     panic!("parse errors: {:?}", parse.errors());
    // }
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
    // The result_type field holds the type of the last expression in the program.
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

// ── Literal Inference ──────────────────────────────────────────────────

#[test]
fn test_integer_literal_is_int() {
    let result = check_source("42");
    assert_result_type(&result, Ty::int());
}

#[test]
fn test_float_literal_is_float() {
    let result = check_source("3.14");
    assert_result_type(&result, Ty::float());
}

#[test]
fn test_string_literal_is_string() {
    let result = check_source("\"hello\"");
    assert_result_type(&result, Ty::string());
}

#[test]
fn test_bool_literal_is_bool() {
    let result = check_source("true");
    assert_result_type(&result, Ty::bool());
}

// ── Let Binding Inference ──────────────────────────────────────────────

#[test]
fn test_let_binding_inference() {
    let result = check_source("let x = 42");
    // After `let x = 42`, the type of the binding should be Int.
    // The result type is the type of the last item/expression.
    assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    // x should be inferred as Int in the type table.
    // We check the result_type which should be the type of the let-binding's initializer.
    let ty = result.result_type.as_ref().expect("expected a result type");
    assert_eq!(format!("{}", ty), "Int");
}

#[test]
fn test_let_binding_with_usage() {
    let result = check_source("let x = 1\nx + 2");
    assert_result_type(&result, Ty::int());
}

#[test]
fn test_refutable_tuple_let_pattern_is_rejected() {
    let result = check_source("let (0, value) = (0, 42)\nvalue");
    assert_has_error(
        &result,
        |error| {
            matches!(
                error,
                TypeError::InvalidLetPattern { reason, .. }
                    if reason.contains("must match every value")
            )
        },
        "InvalidLetPattern(refutable)",
    );
}

#[test]
fn test_tuple_let_pattern_rejects_duplicate_binders() {
    let result = check_source("let (value, value) = (1, 2)\nvalue");
    assert_has_error(
        &result,
        |error| {
            matches!(
                error,
                TypeError::InvalidLetPattern { reason, .. }
                    if reason.contains("must be unique")
            )
        },
        "InvalidLetPattern(duplicate binder)",
    );
}

/// A binder must start with a lowercase letter (an uppercase name is a
/// constructor). The pattern's error is the only one: its other names are
/// still bound, where they used to be "undefined variable" at each use.
#[test]
fn an_invalid_let_pattern_is_its_only_error() {
    let result = check_source("fn main() do\n  let (A, b) = (1, 2)\n  b + 1\nend");
    assert_eq!(result.errors.len(), 1, "{:?}", result.errors);
    assert!(
        matches!(
            &result.errors[0],
            TypeError::InvalidLetPattern { reason, .. } if reason.contains("lowercase")
        ),
        "{:?}",
        result.errors
    );
}

// ── Function Inference ─────────────────────────────────────────────────

#[test]
fn test_function_identity() {
    let result = check_source("let id = fn (x) -> x end\nid(1)");
    assert_result_type(&result, Ty::int());
}

/// SUCCESS CRITERION #1: Let-polymorphism
/// A let-bound value is generalized, so a named identity function bound to a
/// local can be used at multiple types.
#[test]
fn test_let_polymorphism() {
    let result = check_source(
        "fn ident(x) do\n  x\nend\nlet id = ident\nlet a = id(1)\nlet b = id(\"hello\")\nb",
    );
    assert!(
        result.errors.is_empty(),
        "let-polymorphism should not produce errors, got: {:?}",
        result.errors
    );
    // b should be String (the last binding).
    assert_result_type(&result, Ty::string());
}

// ── Occurs Check ───────────────────────────────────────────────────────

/// SUCCESS CRITERION #2: Occurs check rejects self-application.
#[test]
fn test_occurs_check_rejection() {
    let result = check_source("fn (x) -> x(x) end");
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::InfiniteType { .. }),
        "InfiniteType",
    );
}

// ── If Expression ──────────────────────────────────────────────────────

#[test]
fn test_if_branch_mismatch() {
    let result = check_source("if true do 1 else \"hello\" end");
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch (if-branches)",
    );
}

#[test]
fn test_if_branches_same_type() {
    let result = check_source("if true do 1 else 2 end");
    assert_result_type(&result, Ty::int());
}

// ── Arity and Unbound Variable Errors ──────────────────────────────────

#[test]
fn test_function_application_wrong_arity() {
    let result = check_source("let f = fn (x, y) -> x end\nf(1)");
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::ArityMismatch { .. }),
        "ArityMismatch",
    );
}

#[test]
fn test_unbound_variable() {
    let result = check_source("x + 1");
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::UnboundVariable { name, .. } if name == "x"),
        "UnboundVariable(x)",
    );
}

// ── Arithmetic and Comparison ──────────────────────────────────────────

#[test]
fn test_arithmetic_int() {
    let result = check_source("1 + 2 * 3");
    assert_result_type(&result, Ty::int());
}

#[test]
fn test_comparison_returns_bool() {
    let result = check_source("1 < 2");
    assert_result_type(&result, Ty::bool());
}

// ── Nested Function Inference ──────────────────────────────────────────

#[test]
fn test_nested_function_inference() {
    let result = check_source("let apply = fn (f, x) -> f(x) end\napply(fn (n) -> n + 1 end, 42)");
    assert_result_type(&result, Ty::int());
}

// ── Tuple accessors ────────────────────────────────────────────────────

#[test]
fn test_computed_tuple_index_needs_one_element_type() {
    // Any element could be the one selected, so a computed index on a tuple
    // of mixed types has no knowable result type. Accepting it would read a
    // reference's slot as the declared `Int`, handing back its address.
    let result = check_source("let t = (1, \"two\", 3)\nlet i = 1 + 0\nTuple.nth(t, i)");
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch (computed index into a mixed tuple)",
    );
}

#[test]
fn test_computed_tuple_index_takes_the_shared_element_type() {
    let result = check_source("let t = (\"a\", \"b\")\nlet i = 1 + 0\nTuple.nth(t, i)");
    assert_result_type(&result, Ty::string());
}

// ── Json where a String is expected ──────────────────────────────────

#[test]
fn test_json_arguments_convert_to_strings() {
    // Written and piped arguments, to a module function and a user one: the
    // checker records each, and lowering passes its encoded text.
    let result = check_source(
        "fn shout(s :: String) -> String do\n  String.to_upper(s)\nend\n\
         let j = json { a: 1 }\n\
         println(j)\n\
         shout(j)\n\
         j |> String.length",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.json_text_arguments.len(), 3);
}

#[test]
fn test_json_is_no_string_outside_an_argument() {
    // An annotation, a list's elements, and a String where a Json goes.
    for src in [
        "let s :: String = json { a: 1 }\ns",
        "let xs :: List<String> = [json { a: 1 }]\nxs",
        "Json.object_get(\"{}\", \"a\")",
    ] {
        let result = check_source(src);
        assert_has_error(
            &result,
            |e| matches!(e, TypeError::Mismatch { .. }),
            "Mismatch (Json and String)",
        );
        assert!(result.json_text_arguments.is_empty(), "{src}");
    }
}

#[test]
fn test_a_builtin_type_names_no_value() {
    // It type checked as a value of the type, and lowering failed on it.
    for name in ["Int", "Map", "Json"] {
        let result = check_source(&format!("let v = {name}\nv"));
        assert_has_error(
            &result,
            |e| matches!(e, TypeError::TypeNotValue { name: n, builtin: true, .. } if n == name),
            "TypeNotValue (a built-in type)",
        );
    }
    // Its module's functions are still reached through it.
    let result = check_source("String.length(\"abc\")");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn test_a_structural_impl_needs_its_elements_to_have_the_trait() {
    // Functions have no Eq, so neither has an Option, list, tuple or map
    // value holding them; lowering refused these without a location.
    let f = "fn f(x :: Int) -> Int do\n  x\nend\n";
    for (src, trait_name) in [
        ("Some(f) == Some(f)", "Eq"),
        ("[f] == [f]", "Eq"),
        ("(1, f) == (1, f)", "Eq"),
        ("Map.put(Map.new(), \"k\", f) == Map.new()", "Eq"),
        ("\"#{Some(f)}\"", "Display"),
    ] {
        let result = check_source(&format!("{f}{src}"));
        assert_has_error(
            &result,
            |e| matches!(e, TypeError::TraitNotSatisfied { trait_name: t, .. } if t == trait_name),
            src,
        );
    }
    // Elements that have it.
    let result = check_source("[[1]] == [[1]] && Some((1, \"a\")) == Some((1, \"a\"))");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn test_a_generic_from_json_needs_json_of_its_instantiation() {
    // It failed at run time: "cannot decode Point from JSON".
    let prelude = "struct Point do\n  x :: Int\nend\n\nstruct Box<T> do\n  value :: T\nend deriving(Json)\n\n";
    let result = check_source(&format!(
        "{prelude}let r :: Result<Box<Point>, String> = Box.from_json(\"{{}}\")\nr"
    ));
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::TraitNotSatisfied { trait_name, .. } if trait_name == "Json"),
        "TraitNotSatisfied Json (Box<Point>)",
    );
    let result = check_source(&format!(
        "{prelude}let r :: Result<Box<Int>, String> = Box.from_json(\"{{}}\")\nr"
    ));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn test_a_key_needs_eq_where_it_is_added() {
    // A map's keys and a set's elements are compared by their Eq; a set of
    // functions was refused by lowering, with no place.
    let f = "fn f(x :: Int) -> Int do\n  x\nend\n";
    for src in [
        "Set.size(Set.add(Set.new(), Some(f)))",
        "Map.size(Map.put(Map.new(), (f, 1), 2))",
    ] {
        let result = check_source(&format!("{f}{src}"));
        let eq_errors = result
            .errors
            .iter()
            .filter(|e| matches!(e, TypeError::TraitNotSatisfied { trait_name, .. } if trait_name == "Eq"))
            .count();
        assert_eq!(eq_errors, 1, "{src}: {:?}", result.errors);
    }
    let result = check_source("Map.get(Map.put(Map.new(), Some(1), \"one\"), Some(1))");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn test_a_decode_nothing_fixes_is_unknown() {
    // It failed at run time: "cannot decode ?9 from JSON".
    let prelude = "struct Box<T> do\n  value :: T\nend deriving(Json)\n\n";
    let result = check_source(&format!(
        "{prelude}fn main() do\n  let r = Box.from_json(\"{{}}\")\n  println(\"x\")\nend\n"
    ));
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::DecodeTypeUnknown { .. }),
        "DecodeTypeUnknown",
    );
    // A generic function's callers fix it; so does a use of the value.
    let result = check_source(&format!(
        "{prelude}fn parse(text :: String) do\n  Box.from_json(text)\nend\n\n\
         fn main() do\n  let r :: Result<Box<Int>, String> = parse(\"{{}}\")\n  \
         case Box.from_json(\"{{}}\") do\n    Ok(b) -> b.value + 1\n    Err(_) -> 0\n  end\nend\n"
    ));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn test_a_derive_needs_its_fields_to_have_the_trait() {
    // A derived Eq, Ord, Display or Debug calls the field's own: a field
    // without it failed to build (LLVM verification, "cannot convert a
    // value of type `Point` to a string").
    let point = "struct Point do\n  x :: Int\nend deriving()\n\n";
    for (derive, trait_name) in [("Eq", "Eq"), ("Display", "Display"), ("Debug", "Debug")] {
        let result = check_source(&format!(
            "{point}type T do\n  A(Point)\nend deriving({derive})\n"
        ));
        assert_has_error(
            &result,
            |e| matches!(e, TypeError::UnderivableFieldType { trait_name: t, .. } if t == trait_name),
            derive,
        );
    }
    // Derived by default (no deriving clause), the trait is quietly left
    // out: the type has no Eq to use.
    let result = check_source(&format!(
        "{point}struct H do\n  p :: Point\nend\n\nfn main() do\n  let h = H {{ p: Point {{ x: 1 }} }}\n  h == h\nend\n"
    ));
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::TraitNotSatisfied { trait_name, .. } if trait_name == "Eq"),
        "H has no Eq",
    );
    assert!(!result
        .errors
        .iter()
        .any(|e| matches!(e, TypeError::UnderivableFieldType { .. })));
    // A field of a type declared later, or implemented by hand, has it.
    let result = check_source(
        "struct Later do\n  q :: Q\nend deriving(Eq, Display)\n\n\
         struct Q do\n  n :: Int\nend deriving(Eq)\n\n\
         impl Display for Q do\n  fn to_string(self) -> String do\n    \"q\"\n  end\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn test_showing_a_value_without_debug_names_the_trait() {
    // It was "undefined variable `inspect`".
    let result = check_source(
        "struct Point do\n  x :: Int\nend deriving()\n\nfn main() do\n  inspect(Some(Point { x: 1 }))\nend\n",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::TraitNotSatisfied { trait_name, .. } if trait_name == "Debug"),
        "Debug",
    );
}

#[test]
fn test_a_json_literal_holds_only_json() {
    // A struct deriving nothing was written as `null`.
    let result = check_source(
        "struct Point do\n  x :: Int\nend deriving()\n\nfn main() do\n  json { p: Point { x: 1 }, n: nil }\nend\n",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::TraitNotSatisfied { trait_name, .. } if trait_name == "Json"),
        "Json",
    );
    let result = check_source("fn main() do\n  json { n: nil, xs: [1], o: Some(\"a\") }\nend\n");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

// ── Callbacks returning () ─────────────────────────────────────────────

#[test]
fn test_unit_callback_accepts_a_handler_ending_in_a_value() {
    // `on_message` ends in `Ws.broadcast`, which returns the failure count;
    // `Ws.serve` wants a handler returning (), and discards the count.
    let result = check_source(
        "fn on_connect(conn, _path, _headers) -> Int do\n  Ws.join(conn, \"room\")\n  1\nend\n\
         fn on_message(_conn, msg :: String) do\n  Ws.broadcast(\"room\", msg)\nend\n\
         fn on_close(_conn, _code, _reason) do\n  println(\"closed\")\nend\n\
         Ws.serve(on_connect, on_message, on_close, 9001)",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.discarded_callback_results.len(), 1);
}

#[test]
fn test_unit_callback_accepts_a_closure_ending_in_a_value() {
    let result = check_source(
        "fn each(f :: Fun(Int) -> ()) do\n  f(1)\nend\n\
         let base = 10\n\
         each(fn n -> base + n end)",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.discarded_callback_results.len(), 1);
}

#[test]
fn test_unit_callback_still_checks_parameters() {
    let result = check_source(
        "fn each(f :: Fun(Int) -> ()) do\n  f(1)\nend\n\
         fn shout(s :: String) -> String do\n  s\nend\n\
         each(shout)",
    );
    assert_has_error(
        &result,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch (callback parameter String against Int)",
    );
}

/// A generic type named without its arguments takes inferred ones: `List`
/// in an annotation is `List<_>`, not a type that no value has.
#[test]
fn test_bare_generic_annotation_infers_its_arguments() {
    let result = check_source(
        "fn count(xs :: List) -> Int do\n  List.length(xs)\nend\n\
         fn first(o :: Option) -> Bool do\n  case o do\n    Some(_) -> true\n    None -> false\n  end\nend\n\
         let m :: Map = %{\"a\" => 1}\n\
         let n = count([1, 2]) + count([\"a\"])\n\
         first(Some(n))",
    );
    assert_result_type(&result, Ty::bool());
}

/// An expression the parser could not complete has its parse error and no
/// type error besides. An operator without its operand used to add
/// "expected Never, found Never", and a field access without its name
/// "type `Int` has no field `<unknown>`".
#[test]
fn an_unfinished_expression_has_no_type_error_of_its_own() {
    for src in [
        "fn main() do\n  let x = 1 +\nend\n",
        "fn main() do\n  let x = 1 |>\nend\n",
        "fn main() do\n  let x = -\nend\n",
        "fn main() do\n  let x =\nend\n",
        "fn main() do\n  let x = 1\n  let y = x.\nend\n",
    ] {
        let parse = mesh_parser::parse(src);
        assert!(!parse.errors().is_empty(), "{src}");
        let result = mesh_typeck::check(&parse);
        assert!(result.errors.is_empty(), "{src}: {:?}", result.errors);
    }
}

/// A parameter's annotation holds in a function written `fn f(x :: Int) =
/// ...`, with a guard, or in several clauses, as in a `do` body: it was
/// ignored there, and `id_int("a")` compiled.
#[test]
fn clause_parameter_annotations_are_checked() {
    for def in [
        "fn id_int(x :: Int) = x",
        "fn id_int(x :: Int) when x > 0 do\n  x\nend\nfn id_int(x :: Int) = 0 - x",
    ] {
        let src = format!("{def}\n\nfn main() do\n  id_int(\"a\")\nend\n");
        let result = mesh_typeck::check(&mesh_parser::parse(&src));
        assert!(
            result.errors.iter().any(|error| matches!(
                error,
                mesh_typeck::error::TypeError::Mismatch { expected, found, .. }
                    if *expected == Ty::int() && *found == Ty::string()
            )),
            "{def}: {:?}",
            result.errors
        );
    }
}

/// A function's clauses are one function when they are consecutive:
/// another definition between them is an error. Visibility, generics, a
/// return type or a where-clause belong on the first clause; on a later
/// one they are ignored, with a warning. Clauses of another arity are
/// another function, not a mismatch.
#[test]
fn clause_groups_are_consecutive_and_annotated_on_their_first_clause() {
    let check = |defs: &str| {
        mesh_typeck::check(&mesh_parser::parse(&format!(
            "{defs}\n\nfn main() do\n  f(1)\nend\n"
        )))
    };
    let split = check("fn f(0) = 1\nfn g() = 2\nfn f(n) = n");
    assert!(
        split.errors.iter().any(|error| matches!(
            error,
            mesh_typeck::error::TypeError::NonConsecutiveClauses { fn_name, arity: 1, .. } if fn_name == "f"
        )),
        "{:?}",
        split.errors
    );
    for (defs, what) in [
        ("fn f(0) = 1\npub fn f(n) = n", "visibility"),
        ("fn f(0) = 1\nfn f<T>(n) = 2", "generic parameters"),
        (
            "fn f(0) -> Int = 1\nfn f(n) -> Int = n",
            "return type annotation",
        ),
        ("fn f(0) = 1\nfn f(n) where n: Display = 2", "where clause"),
    ] {
        let result = check(defs);
        assert!(result.errors.is_empty(), "{defs}: {:?}", result.errors);
        assert!(
            result.warnings.iter().any(|warning| matches!(
                warning,
                mesh_typeck::error::TypeError::NonFirstClauseAnnotation { what: found, .. } if found == what
            )),
            "{defs}: {:?}",
            result.warnings
        );
    }
    let overloads = check("fn f(0) = 1\nfn f(n) = n\nfn f(a, b) = a + b");
    assert!(overloads.errors.is_empty(), "{:?}", overloads.errors);
}

/// Only `let` binds inside a function. A nested `fn` type-checked and then
/// failed in code generation ("Undefined variable"), and a nested `struct`,
/// `import` (even of no module), `type` or `actor` was silently ignored.
#[test]
fn definitions_inside_a_function_are_refused() {
    for (body, keyword) in [
        ("fn helper(x) do\n    x + 1\n  end", "fn"),
        ("fn sign(0) = 0\n  fn sign(n) = 1", "fn"),
        ("struct P do\n    x :: Int\n  end", "struct"),
        ("import Nowhere", "import"),
        ("type Alias = Int", "type"),
        (
            "actor a() do\n    receive do\n      _ -> nil\n    end\n  end",
            "actor",
        ),
        ("module Inner do\n    fn x() = 1\n  end", "module"),
        ("from Nowhere import x", "from ... import"),
        ("interface I do\n    fn f(self) -> Int\n  end", "interface"),
        ("impl Display for Int do\n  end", "impl"),
        ("type Shape do\n    Dot\n  end", "type"),
        (
            "service S do\n    fn init() -> Int do\n      0\n    end\n  end",
            "service",
        ),
        (
            "supervisor Sup do\n    strategy: one_for_one\n  end",
            "supervisor",
        ),
    ] {
        let src = format!("fn main() do\n  {body}\n  1\nend\n");
        let result = mesh_typeck::check(&mesh_parser::parse(&src));
        assert!(
            result.errors.iter().any(|error| matches!(
                error,
                TypeError::NestedDefinition { keyword: found, .. } if *found == keyword
            )),
            "{body}: {:?}",
            result.errors
        );
    }
    let fine = mesh_typeck::check(&mesh_parser::parse(
        "fn main() do\n  let helper = fn x -> x + 1 end\n  helper(1)\nend\n",
    ));
    assert!(fine.errors.is_empty(), "{:?}", fine.errors);
}

/// A `for` over a `Set<T>` binds `T`, whatever `T` is (the iterators guide
/// said a set's loop variable is an `Int`).
#[test]
fn a_for_over_a_set_binds_its_element_type() {
    let result = check_source(
        "fn main() do\n  for s in Set.from_list([\"a\"]) do\n    String.length(s)\n  end\nend\n",
    );
    assert_result_type(&result, Ty::fun(vec![], Ty::list(Ty::int())));
}

/// A method whose return type its body settles is found by a call above
/// its impl, as a function is above its definition: the impl was checked
/// in its place, after the call, and the call was "no method".
#[test]
fn a_method_is_found_above_its_impl() {
    let result = check_source(
        "fn main() do\n  let b = Box { items: [1] }\n  b.count() + b.twice() + List.length(b.listed())\nend\n\n\
         interface Counter do\n  fn count(self) -> Int\nend\n\n\
         interface Twice do\n  fn twice(self)\nend\n\n\
         interface Lister do\n  fn listed(self) -> List\nend\n\n\
         struct Box do\n  items :: List<Int>\nend\n\n\
         impl Counter for Box do\n  fn count(self) do\n    List.length(self.items)\n  end\nend\n\n\
         impl Twice for Box do\n  fn twice(self) do\n    List.length(self.items) * 2\n  end\nend\n\n\
         impl Lister for Box do\n  fn listed(self) -> List do\n    self.items\n  end\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// `Unit`, as the documentation writes it, is `()`: a function annotated
/// `-> Unit` returns what `println` does, and `Result<Unit, String>` holds
/// `nil`. It was a type of its own that nothing had.
#[test]
fn unit_is_the_empty_tuple() {
    let result = check_source(
        "fn quiet() -> Unit do\n  println(\"quiet\")\nend\n\nfn done() -> Result<Unit, String> do\n  quiet()\n  Ok(nil)\nend\n\nfn main() do\n  done()\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

/// An `Int`, a `Float` and a `Bytes` take their modules' functions as
/// methods, as a `String` takes `String`'s: `5.to_float()` is
/// `Int.to_float(5)`. Only `String` and `Range` values did.
#[test]
fn numbers_and_bytes_take_their_modules_functions_as_methods() {
    let result = check_source(
        "fn main() do\n  let f = 5.to_float()\n  let i = 3.7.to_int()\n  let n = Bytes.from_utf8(\"abc\").length()\n  f + Int.to_float(i + n)\nend\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_result_type(&result, Ty::fun(vec![], Ty::float()));
}
