//! Built-in type registration.
//!
//! Registers primitive types (Int, Float, String, Bool), generic type
//! constructors (Option, Result), built-in arithmetic operators, and
//! compiler-known traits (Add, Sub, Mul, Div, Mod, Eq, Ord, Not) into
//! the type environment and trait registry.

use rustc_hash::FxHashMap;

use crate::env::TypeEnv;
use crate::traits::{ImplDef, ImplMethodSig, TraitDef, TraitMethodSig, TraitRegistry};
use crate::ty::{Scheme, Ty, TyCon, TyVar};
use crate::unify::InferCtx;

/// Register all built-in types and functions into the environment.
///
/// After this call, the environment contains:
/// - Primitive types: Int, Float, String, Bool
/// - Generic constructors: Option (arity 1), Result (arity 2)
/// - Arithmetic operators: +, -, *, / for Int and Float
/// - Comparison operators (returns Bool)
/// - Logical operators (and, or, not)
///
/// And the trait registry contains compiler-known traits:
/// - Add, Sub, Mul, Div, Mod (numeric operations)
/// - Eq (equality comparison)
/// - Ord (ordering comparison)
/// - Not (logical negation)
pub fn register_builtins(
    _ctx: &mut InferCtx,
    env: &mut TypeEnv,
    trait_registry: &mut TraitRegistry,
) {
    // ── Primitive type constructors ─────────────────────────────────

    // These are registered as monomorphic schemes so they can be
    // referenced as type names in annotations.
    env.insert("Int".into(), Scheme::mono(Ty::int()));
    env.insert("Float".into(), Scheme::mono(Ty::float()));
    env.insert("String".into(), Scheme::mono(Ty::string()));
    env.insert("Bool".into(), Scheme::mono(Ty::bool()));

    // ── Actor type constructor ────────────────────────────────────
    //
    // Pid is a type constructor with arity 1: Pid<M> where M is the
    // message type. Untyped Pid (arity 0) is also valid as an escape
    // hatch. We register the bare name so `Pid` resolves in annotations.
    env.insert("Pid".into(), Scheme::mono(Ty::untyped_pid()));

    // ── I/O builtins ──────────────────────────────────────────────

    // println(String) -> () -- prints a string with trailing newline
    env.insert(
        "println".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::Tuple(vec![]))),
    );

    // print(String) -> () -- prints a string without trailing newline
    env.insert(
        "print".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::Tuple(vec![]))),
    );

    // ── Compiler-known traits ──────────────────────────────────────

    register_compiler_known_traits(trait_registry);

    // ── Arithmetic operators (Int) ──────────────────────────────────

    // These remain in the env for backward compatibility. The binary
    // operator inference also uses trait dispatch for type checking.
    let int_binop = Scheme::mono(Ty::fun(vec![Ty::int(), Ty::int()], Ty::int()));
    env.insert("+".into(), int_binop.clone());
    env.insert("-".into(), int_binop.clone());
    env.insert("*".into(), int_binop.clone());
    env.insert("/".into(), int_binop);

    // ── Float arithmetic ────────────────────────────────────────────

    let float_binop = Scheme::mono(Ty::fun(
        vec![Ty::float(), Ty::float()],
        Ty::float(),
    ));
    env.insert("+.".into(), float_binop.clone());
    env.insert("-.".into(), float_binop.clone());
    env.insert("*.".into(), float_binop.clone());
    env.insert("/.".into(), float_binop);

    // ── String concatenation ────────────────────────────────────────

    env.insert(
        "<>".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::string()], Ty::string())),
    );

    // ── Comparison (returns Bool) ───────────────────────────────────

    let int_cmp = Scheme::mono(Ty::fun(vec![Ty::int(), Ty::int()], Ty::bool()));
    env.insert("==".into(), int_cmp.clone());
    env.insert("!=".into(), int_cmp.clone());
    env.insert("<".into(), int_cmp.clone());
    env.insert(">".into(), int_cmp.clone());
    env.insert("<=".into(), int_cmp.clone());
    env.insert(">=".into(), int_cmp);

    // ── Logical operators ───────────────────────────────────────────

    let bool_binop = Scheme::mono(Ty::fun(vec![Ty::bool(), Ty::bool()], Ty::bool()));
    env.insert("and".into(), bool_binop.clone());
    env.insert("or".into(), bool_binop);
    env.insert(
        "not".into(),
        Scheme::mono(Ty::fun(vec![Ty::bool()], Ty::bool())),
    );

    // ── Standard library: String operations (Phase 8) ────────────────

    env.insert(
        "string_length".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::int())),
    );
    env.insert(
        "string_slice".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::int(), Ty::int()], Ty::string())),
    );
    env.insert(
        "string_contains".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::string()], Ty::bool())),
    );
    env.insert(
        "string_starts_with".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::string()], Ty::bool())),
    );
    env.insert(
        "string_ends_with".into(),
        Scheme::mono(Ty::fun(vec![Ty::string(), Ty::string()], Ty::bool())),
    );
    env.insert(
        "string_trim".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::string())),
    );
    env.insert(
        "string_to_upper".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::string())),
    );
    env.insert(
        "string_to_lower".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::string())),
    );
    env.insert(
        "string_replace".into(),
        Scheme::mono(Ty::fun(
            vec![Ty::string(), Ty::string(), Ty::string()],
            Ty::string(),
        )),
    );

    // ── Standard library: File I/O functions (Phase 8) ──────────────────

    env.insert(
        "file_read".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::result(Ty::string(), Ty::string()))),
    );
    env.insert(
        "file_write".into(),
        Scheme::mono(Ty::fun(
            vec![Ty::string(), Ty::string()],
            Ty::result(Ty::Tuple(vec![]), Ty::string()),
        )),
    );
    env.insert(
        "file_append".into(),
        Scheme::mono(Ty::fun(
            vec![Ty::string(), Ty::string()],
            Ty::result(Ty::Tuple(vec![]), Ty::string()),
        )),
    );
    env.insert(
        "file_exists".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::bool())),
    );
    env.insert(
        "file_delete".into(),
        Scheme::mono(Ty::fun(
            vec![Ty::string()],
            Ty::result(Ty::Tuple(vec![]), Ty::string()),
        )),
    );

    // ── Standard library: IO functions (Phase 8) ─────────────────────

    env.insert(
        "io_read_line".into(),
        Scheme::mono(Ty::fun(vec![], Ty::result(Ty::string(), Ty::string()))),
    );
    env.insert(
        "io_eprintln".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::Tuple(vec![]))),
    );

    // ── Standard library: Env functions (Phase 8) ────────────────────

    env.insert(
        "env_get".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::option(Ty::string()))),
    );

    // ── Standard library: Collection types (Phase 8) ─────────────────

    // Type constructors for collection types (bare names).
    env.insert("List".into(), Scheme::mono(Ty::list_untyped()));
    env.insert("Map".into(), Scheme::mono(Ty::map_untyped()));
    env.insert("Set".into(), Scheme::mono(Ty::set_untyped()));
    env.insert("Range".into(), Scheme::mono(Ty::range()));
    env.insert("Queue".into(), Scheme::mono(Ty::queue()));

    // Use opaque types (untyped) for collection function signatures.
    // At the LLVM level these are all pointers; type safety is checked by Snow's type system.
    let list_t = Ty::list_untyped();
    let map_t = Ty::map_untyped();
    let set_t = Ty::set_untyped();
    let range_t = Ty::range();
    let queue_t = Ty::queue();

    // Closure type for higher-order functions: (Int) -> Int (monomorphic fallback).
    let int_to_int = Ty::fun(vec![Ty::int()], Ty::int());
    let int_to_bool = Ty::fun(vec![Ty::int()], Ty::bool());
    let int_int_to_int = Ty::fun(vec![Ty::int(), Ty::int()], Ty::int());

    // ── Prelude (bare names): map, filter, reduce, head, tail ─────────
    // These are auto-imported without module qualification.

    // map(list, fn) -> list  (bare prelude name)
    env.insert(
        "map".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), int_to_int.clone()], list_t.clone())),
    );
    // filter(list, fn) -> list
    env.insert(
        "filter".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), int_to_bool.clone()], list_t.clone())),
    );
    // reduce(list, init, fn) -> Int
    env.insert(
        "reduce".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), Ty::int(), int_int_to_int.clone()], Ty::int())),
    );
    // head(list) -> Int
    env.insert(
        "head".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone()], Ty::int())),
    );
    // tail(list) -> list
    env.insert(
        "tail".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone()], list_t.clone())),
    );

    // Also register with list_ prefix for module-qualified lowering.
    env.insert(
        "list_map".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), int_to_int.clone()], list_t.clone())),
    );
    env.insert(
        "list_filter".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), int_to_bool.clone()], list_t.clone())),
    );
    env.insert(
        "list_reduce".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), Ty::int(), int_int_to_int.clone()], Ty::int())),
    );
    env.insert(
        "list_head".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone()], Ty::int())),
    );
    env.insert(
        "list_tail".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone()], list_t.clone())),
    );

    // ── List module functions ─────────────────────────────────────────

    // List.new() -> List
    env.insert(
        "list_new".into(),
        Scheme::mono(Ty::fun(vec![], list_t.clone())),
    );
    // List.length(list) -> Int
    env.insert(
        "list_length".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone()], Ty::int())),
    );
    // List.append(list, element) -> List
    env.insert(
        "list_append".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), Ty::int()], list_t.clone())),
    );
    // List.get(list, index) -> Int
    env.insert(
        "list_get".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), Ty::int()], Ty::int())),
    );
    // List.concat(a, b) -> List
    env.insert(
        "list_concat".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone(), list_t.clone()], list_t.clone())),
    );
    // List.reverse(list) -> List
    env.insert(
        "list_reverse".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone()], list_t.clone())),
    );

    // ── Map module functions (polymorphic) ──────────────────────────────
    {
        let k_var = TyVar(90000);
        let v_var = TyVar(90001);
        let k = Ty::Var(k_var);
        let v = Ty::Var(v_var);
        let map_kv = Ty::map(k.clone(), v.clone());

        env.insert("map_new".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![], map_kv.clone()) });
        env.insert("map_put".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![map_kv.clone(), k.clone(), v.clone()], map_kv.clone()) });
        env.insert("map_get".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![map_kv.clone(), k.clone()], v.clone()) });
        env.insert("map_has_key".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![map_kv.clone(), k.clone()], Ty::bool()) });
        env.insert("map_delete".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![map_kv.clone(), k.clone()], map_kv.clone()) });
        env.insert("map_size".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![map_kv.clone()], Ty::int()) });
        env.insert("map_keys".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![map_kv.clone()], list_t.clone()) });
        env.insert("map_values".into(), Scheme { vars: vec![k_var, v_var], ty: Ty::fun(vec![map_kv.clone()], list_t.clone()) });
    }

    // ── Set module functions ──────────────────────────────────────────

    env.insert(
        "set_new".into(),
        Scheme::mono(Ty::fun(vec![], set_t.clone())),
    );
    env.insert(
        "set_add".into(),
        Scheme::mono(Ty::fun(vec![set_t.clone(), Ty::int()], set_t.clone())),
    );
    env.insert(
        "set_remove".into(),
        Scheme::mono(Ty::fun(vec![set_t.clone(), Ty::int()], set_t.clone())),
    );
    env.insert(
        "set_contains".into(),
        Scheme::mono(Ty::fun(vec![set_t.clone(), Ty::int()], Ty::bool())),
    );
    env.insert(
        "set_size".into(),
        Scheme::mono(Ty::fun(vec![set_t.clone()], Ty::int())),
    );
    env.insert(
        "set_union".into(),
        Scheme::mono(Ty::fun(vec![set_t.clone(), set_t.clone()], set_t.clone())),
    );
    env.insert(
        "set_intersection".into(),
        Scheme::mono(Ty::fun(vec![set_t.clone(), set_t.clone()], set_t.clone())),
    );

    // ── Tuple module functions ────────────────────────────────────────

    env.insert(
        "tuple_nth".into(),
        Scheme::mono(Ty::fun(vec![Ty::Con(TyCon::new("Tuple")), Ty::int()], Ty::int())),
    );
    env.insert(
        "tuple_first".into(),
        Scheme::mono(Ty::fun(vec![Ty::Con(TyCon::new("Tuple"))], Ty::int())),
    );
    env.insert(
        "tuple_second".into(),
        Scheme::mono(Ty::fun(vec![Ty::Con(TyCon::new("Tuple"))], Ty::int())),
    );
    env.insert(
        "tuple_size".into(),
        Scheme::mono(Ty::fun(vec![Ty::Con(TyCon::new("Tuple"))], Ty::int())),
    );

    // ── Range module functions ────────────────────────────────────────

    env.insert(
        "range_new".into(),
        Scheme::mono(Ty::fun(vec![Ty::int(), Ty::int()], range_t.clone())),
    );
    env.insert(
        "range_to_list".into(),
        Scheme::mono(Ty::fun(vec![range_t.clone()], list_t.clone())),
    );
    env.insert(
        "range_map".into(),
        Scheme::mono(Ty::fun(vec![range_t.clone(), int_to_int.clone()], list_t.clone())),
    );
    env.insert(
        "range_filter".into(),
        Scheme::mono(Ty::fun(vec![range_t.clone(), int_to_bool], list_t.clone())),
    );
    env.insert(
        "range_length".into(),
        Scheme::mono(Ty::fun(vec![range_t.clone()], Ty::int())),
    );

    // ── Queue module functions ────────────────────────────────────────

    env.insert(
        "queue_new".into(),
        Scheme::mono(Ty::fun(vec![], queue_t.clone())),
    );
    env.insert(
        "queue_push".into(),
        Scheme::mono(Ty::fun(vec![queue_t.clone(), Ty::int()], queue_t.clone())),
    );
    env.insert(
        "queue_pop".into(),
        Scheme::mono(Ty::fun(vec![queue_t.clone()], Ty::Con(TyCon::new("Tuple")))),
    );
    env.insert(
        "queue_peek".into(),
        Scheme::mono(Ty::fun(vec![queue_t.clone()], Ty::int())),
    );
    env.insert(
        "queue_size".into(),
        Scheme::mono(Ty::fun(vec![queue_t.clone()], Ty::int())),
    );
    env.insert(
        "queue_is_empty".into(),
        Scheme::mono(Ty::fun(vec![queue_t.clone()], Ty::bool())),
    );

    // ── Standard library: JSON functions (Phase 8 Plan 04) ────────────

    // Json type is opaque (Ptr) -- pattern matching on Json sum type deferred
    let json_t = Ty::Con(TyCon::new("Json"));
    env.insert("Json".into(), Scheme::mono(json_t.clone()));

    // JSON.parse(string) -> Result<Json, String>
    env.insert(
        "json_parse".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::result(json_t.clone(), Ty::string()))),
    );
    // JSON.encode(json) -> String
    env.insert(
        "json_encode".into(),
        Scheme::mono(Ty::fun(vec![json_t.clone()], Ty::string())),
    );
    // JSON.encode_string(string) -> String
    env.insert(
        "json_encode_string".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::string())),
    );
    // JSON.encode_int(int) -> String
    env.insert(
        "json_encode_int".into(),
        Scheme::mono(Ty::fun(vec![Ty::int()], Ty::string())),
    );
    // JSON.encode_bool(bool) -> String
    env.insert(
        "json_encode_bool".into(),
        Scheme::mono(Ty::fun(vec![Ty::bool()], Ty::string())),
    );
    // JSON.encode_map(map) -> String
    env.insert(
        "json_encode_map".into(),
        Scheme::mono(Ty::fun(vec![map_t.clone()], Ty::string())),
    );
    // JSON.encode_list(list) -> String
    env.insert(
        "json_encode_list".into(),
        Scheme::mono(Ty::fun(vec![list_t.clone()], Ty::string())),
    );

    // ── Standard library: HTTP functions (Phase 8 Plan 05) ────────────

    // HTTP types (opaque, resolve to MirType::Ptr at codegen level)
    let request_t = Ty::Con(TyCon::new("Request"));
    let response_t = Ty::Con(TyCon::new("Response"));
    let router_t = Ty::Con(TyCon::new("Router"));
    env.insert("Request".into(), Scheme::mono(request_t.clone()));
    env.insert("Response".into(), Scheme::mono(response_t.clone()));
    env.insert("Router".into(), Scheme::mono(router_t.clone()));

    // HTTP.router() -> Router
    env.insert(
        "http_router".into(),
        Scheme::mono(Ty::fun(vec![], router_t.clone())),
    );
    // HTTP.route(Router, String, (Request) -> Response) -> Router
    env.insert(
        "http_route".into(),
        Scheme::mono(Ty::fun(
            vec![router_t.clone(), Ty::string(), Ty::fun(vec![request_t.clone()], response_t.clone())],
            router_t.clone(),
        )),
    );
    // HTTP.serve(Router, Int) -> ()
    env.insert(
        "http_serve".into(),
        Scheme::mono(Ty::fun(vec![router_t.clone(), Ty::int()], Ty::Tuple(vec![]))),
    );
    // HTTP.response(Int, String) -> Response
    env.insert(
        "http_response".into(),
        Scheme::mono(Ty::fun(vec![Ty::int(), Ty::string()], response_t.clone())),
    );
    // HTTP.get(String) -> Result<String, String>
    env.insert(
        "http_get".into(),
        Scheme::mono(Ty::fun(vec![Ty::string()], Ty::result(Ty::string(), Ty::string()))),
    );
    // HTTP.post(String, String) -> Result<String, String>
    env.insert(
        "http_post".into(),
        Scheme::mono(Ty::fun(
            vec![Ty::string(), Ty::string()],
            Ty::result(Ty::string(), Ty::string()),
        )),
    );

    // Request accessor functions
    // Request.method(Request) -> String
    env.insert(
        "request_method".into(),
        Scheme::mono(Ty::fun(vec![request_t.clone()], Ty::string())),
    );
    // Request.path(Request) -> String
    env.insert(
        "request_path".into(),
        Scheme::mono(Ty::fun(vec![request_t.clone()], Ty::string())),
    );
    // Request.body(Request) -> String
    env.insert(
        "request_body".into(),
        Scheme::mono(Ty::fun(vec![request_t.clone()], Ty::string())),
    );
    // Request.header(Request, String) -> Option<String>
    env.insert(
        "request_header".into(),
        Scheme::mono(Ty::fun(
            vec![request_t.clone(), Ty::string()],
            Ty::option(Ty::string()),
        )),
    );
    // Request.query(Request, String) -> Option<String>
    env.insert(
        "request_query".into(),
        Scheme::mono(Ty::fun(
            vec![request_t.clone(), Ty::string()],
            Ty::option(Ty::string()),
        )),
    );
}

/// Register compiler-known traits and their built-in implementations.
///
/// These traits back the arithmetic and comparison operators. When the
/// inference engine encounters `a + b`, it checks that the resolved type
/// of `a` has an impl for `Add`.
fn register_compiler_known_traits(registry: &mut TraitRegistry) {
    // ── Arithmetic traits ──────────────────────────────────────────

    let arithmetic_traits = ["Add", "Sub", "Mul", "Div", "Mod"];
    for trait_name in &arithmetic_traits {
        registry.register_trait(TraitDef {
            name: trait_name.to_string(),
            methods: vec![TraitMethodSig {
                name: trait_name.to_lowercase(),
                has_self: true,
                param_count: 1,
                return_type: None, // return type is Self (the implementing type)
            }],
        });

        // Register impls for Int and Float.
        for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float")] {
            let mut methods = FxHashMap::default();
            methods.insert(
                trait_name.to_lowercase(),
                ImplMethodSig {
                    has_self: true,
                    param_count: 1,
                    return_type: Some(ty.clone()),
                },
            );
            let _ = registry.register_impl(ImplDef {
                trait_name: trait_name.to_string(),
                impl_type: ty.clone(),
                impl_type_name: ty_name.to_string(),
                methods,
            });
        }
    }

    // ── Eq trait ────────────────────────────────────────────────────

    registry.register_trait(TraitDef {
        name: "Eq".to_string(),
        methods: vec![TraitMethodSig {
            name: "eq".to_string(),
            has_self: true,
            param_count: 1,
            return_type: Some(Ty::bool()),
        }],
    });

    // Eq impls for Int, Float, String, Bool.
    for (ty, ty_name) in &[
        (Ty::int(), "Int"),
        (Ty::float(), "Float"),
        (Ty::string(), "String"),
        (Ty::bool(), "Bool"),
    ] {
        let mut methods = FxHashMap::default();
        methods.insert(
            "eq".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::bool()),
            },
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Eq".to_string(),
            impl_type: ty.clone(),
            impl_type_name: ty_name.to_string(),
            methods,
        });
    }

    // ── Ord trait ───────────────────────────────────────────────────

    registry.register_trait(TraitDef {
        name: "Ord".to_string(),
        methods: vec![TraitMethodSig {
            name: "cmp".to_string(),
            has_self: true,
            param_count: 1,
            return_type: Some(Ty::bool()),
        }],
    });

    // Ord impls for Int, Float.
    for (ty, ty_name) in &[(Ty::int(), "Int"), (Ty::float(), "Float")] {
        let mut methods = FxHashMap::default();
        methods.insert(
            "cmp".to_string(),
            ImplMethodSig {
                has_self: true,
                param_count: 1,
                return_type: Some(Ty::bool()),
            },
        );
        let _ = registry.register_impl(ImplDef {
            trait_name: "Ord".to_string(),
            impl_type: ty.clone(),
            impl_type_name: ty_name.to_string(),
            methods,
        });
    }

    // ── Not trait ───────────────────────────────────────────────────

    registry.register_trait(TraitDef {
        name: "Not".to_string(),
        methods: vec![TraitMethodSig {
            name: "not".to_string(),
            has_self: true,
            param_count: 0,
            return_type: Some(Ty::bool()),
        }],
    });

    let mut not_methods = FxHashMap::default();
    not_methods.insert(
        "not".to_string(),
        ImplMethodSig {
            has_self: true,
            param_count: 0,
            return_type: Some(Ty::bool()),
        },
    );
    let _ = registry.register_impl(ImplDef {
        trait_name: "Not".to_string(),
        impl_type: Ty::bool(),
        impl_type_name: "Bool".to_string(),
        methods: not_methods,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_register_primitives() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        let mut trait_registry = TraitRegistry::new();
        register_builtins(&mut ctx, &mut env, &mut trait_registry);

        // Check primitive types exist.
        assert!(env.lookup("Int").is_some());
        assert!(env.lookup("Float").is_some());
        assert!(env.lookup("String").is_some());
        assert!(env.lookup("Bool").is_some());
    }

    #[test]
    fn builtins_register_operators() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        let mut trait_registry = TraitRegistry::new();
        register_builtins(&mut ctx, &mut env, &mut trait_registry);

        // Check arithmetic operators exist.
        assert!(env.lookup("+").is_some());
        assert!(env.lookup("-").is_some());
        assert!(env.lookup("*").is_some());
        assert!(env.lookup("/").is_some());

        // Check comparison operators.
        assert!(env.lookup("==").is_some());
        assert!(env.lookup("<").is_some());
    }

    #[test]
    fn builtins_register_compiler_known_traits() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        let mut trait_registry = TraitRegistry::new();
        register_builtins(&mut ctx, &mut env, &mut trait_registry);

        // Check arithmetic traits.
        assert!(trait_registry.get_trait("Add").is_some());
        assert!(trait_registry.get_trait("Sub").is_some());
        assert!(trait_registry.get_trait("Eq").is_some());
        assert!(trait_registry.get_trait("Ord").is_some());

        // Check impls.
        assert!(trait_registry.has_impl("Add", &Ty::int()));
        assert!(trait_registry.has_impl("Add", &Ty::float()));
        assert!(!trait_registry.has_impl("Add", &Ty::string()));
        assert!(trait_registry.has_impl("Eq", &Ty::string()));
    }

    #[test]
    fn builtins_register_stdlib_functions() {
        let mut ctx = InferCtx::new();
        let mut env = TypeEnv::new();
        let mut trait_registry = TraitRegistry::new();
        register_builtins(&mut ctx, &mut env, &mut trait_registry);

        // String operations
        assert!(env.lookup("string_length").is_some());
        assert!(env.lookup("string_slice").is_some());
        assert!(env.lookup("string_contains").is_some());
        assert!(env.lookup("string_starts_with").is_some());
        assert!(env.lookup("string_ends_with").is_some());
        assert!(env.lookup("string_trim").is_some());
        assert!(env.lookup("string_to_upper").is_some());
        assert!(env.lookup("string_to_lower").is_some());
        assert!(env.lookup("string_replace").is_some());

        // File I/O functions
        assert!(env.lookup("file_read").is_some());
        assert!(env.lookup("file_write").is_some());
        assert!(env.lookup("file_append").is_some());
        assert!(env.lookup("file_exists").is_some());
        assert!(env.lookup("file_delete").is_some());

        // IO functions
        assert!(env.lookup("io_read_line").is_some());
        assert!(env.lookup("io_eprintln").is_some());

        // Env functions
        assert!(env.lookup("env_get").is_some());

        // HTTP functions (Phase 8 Plan 05)
        assert!(env.lookup("http_router").is_some());
        assert!(env.lookup("http_route").is_some());
        assert!(env.lookup("http_serve").is_some());
        assert!(env.lookup("http_response").is_some());
        assert!(env.lookup("http_get").is_some());
        assert!(env.lookup("http_post").is_some());

        // Request accessors
        assert!(env.lookup("request_method").is_some());
        assert!(env.lookup("request_path").is_some());
        assert!(env.lookup("request_body").is_some());
        assert!(env.lookup("request_header").is_some());
        assert!(env.lookup("request_query").is_some());
    }
}
