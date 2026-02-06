//! Type resolution: Ty -> MirType conversion.
//!
//! Converts the type checker's `Ty` representation to the concrete `MirType`
//! used in MIR. After type checking, all types should be fully resolved
//! (no remaining type variables).

use snow_typeck::ty::{Ty, TyCon};
use snow_typeck::TypeRegistry;

use super::MirType;

/// Convert a type checker `Ty` to a concrete `MirType`.
///
/// The `type_registry` is used to determine whether a named type is a struct
/// or a sum type. The `is_closure_context` flag indicates whether function types
/// should be treated as closures (true for values that may be closures) or as
/// known function pointers (false for named functions).
///
/// # Panics
///
/// Panics if a `Ty::Var` is encountered, which indicates an unresolved type
/// variable that should not exist after type checking.
pub fn resolve_type(ty: &Ty, registry: &TypeRegistry, is_closure_context: bool) -> MirType {
    match ty {
        Ty::Var(v) => {
            panic!(
                "Unresolved type variable ?{} found during MIR lowering. \
                 This indicates a bug in the type checker.",
                v.0
            );
        }

        Ty::Con(con) => resolve_con(con, registry),

        Ty::Fun(params, ret) => {
            let param_types: Vec<MirType> = params
                .iter()
                .map(|p| resolve_type(p, registry, false))
                .collect();
            let ret_type = Box::new(resolve_type(ret, registry, false));
            if is_closure_context {
                MirType::Closure(param_types, ret_type)
            } else {
                MirType::FnPtr(param_types, ret_type)
            }
        }

        Ty::App(con_ty, args) => resolve_app(con_ty, args, registry),

        Ty::Tuple(elems) => {
            if elems.is_empty() {
                MirType::Unit
            } else {
                let mir_elems: Vec<MirType> = elems
                    .iter()
                    .map(|e| resolve_type(e, registry, false))
                    .collect();
                MirType::Tuple(mir_elems)
            }
        }

        Ty::Never => MirType::Never,
    }
}

/// Resolve a type constructor (no type arguments).
fn resolve_con(con: &TyCon, registry: &TypeRegistry) -> MirType {
    match con.name.as_str() {
        "Int" => MirType::Int,
        "Float" => MirType::Float,
        "Bool" => MirType::Bool,
        "String" => MirType::String,
        "Unit" | "()" => MirType::Unit,
        name => {
            // Check registry: struct or sum type?
            if registry.struct_defs.contains_key(name) {
                MirType::Struct(name.to_string())
            } else if registry.sum_type_defs.contains_key(name) {
                MirType::SumType(name.to_string())
            } else {
                // Could be a type alias that was already resolved, or an unknown type.
                // Default to struct-like reference for now.
                MirType::Struct(name.to_string())
            }
        }
    }
}

/// Resolve a type application (e.g., Option<Int>, Result<T, E>, or user struct/sum).
fn resolve_app(con_ty: &Ty, args: &[Ty], registry: &TypeRegistry) -> MirType {
    // Extract the base name from the constructor.
    let base_name = match con_ty {
        Ty::Con(con) => &con.name,
        _ => return MirType::Ptr, // fallback for complex type expressions
    };

    // For monomorphization: generate a mangled name from base + args
    if args.is_empty() {
        // No-arg application, e.g., Ty::App(Con("Point"), [])
        return resolve_con(
            &TyCon {
                name: base_name.to_string(),
            },
            registry,
        );
    }

    let mangled_name = mangle_type_name(base_name, args, registry);

    // Check if this is a sum type or struct
    if registry.sum_type_defs.contains_key(base_name) {
        MirType::SumType(mangled_name)
    } else if registry.struct_defs.contains_key(base_name) {
        MirType::Struct(mangled_name)
    } else {
        // Fallback: treat as a sum type (Option, Result are sum types)
        MirType::SumType(mangled_name)
    }
}

/// Generate a mangled type name for a monomorphized generic type.
///
/// E.g., `Option<Int>` -> `"Option_Int"`, `Result<Int, String>` -> `"Result_Int_String"`.
pub fn mangle_type_name(base: &str, args: &[Ty], registry: &TypeRegistry) -> String {
    let mut name = base.to_string();
    for arg in args {
        name.push('_');
        name.push_str(&mir_type_suffix(&resolve_type(arg, registry, false)));
    }
    name
}

/// Get a short suffix string for a MirType (used in name mangling).
fn mir_type_suffix(ty: &MirType) -> String {
    match ty {
        MirType::Int => "Int".to_string(),
        MirType::Float => "Float".to_string(),
        MirType::Bool => "Bool".to_string(),
        MirType::String => "String".to_string(),
        MirType::Unit => "Unit".to_string(),
        MirType::Tuple(elems) => {
            let parts: Vec<String> = elems.iter().map(mir_type_suffix).collect();
            format!("Tuple_{}", parts.join("_"))
        }
        MirType::Struct(name) | MirType::SumType(name) => name.clone(),
        MirType::FnPtr(params, ret) => {
            let p: Vec<String> = params.iter().map(mir_type_suffix).collect();
            format!("Fn_{}_to_{}", p.join("_"), mir_type_suffix(ret))
        }
        MirType::Closure(params, ret) => {
            let p: Vec<String> = params.iter().map(mir_type_suffix).collect();
            format!("Closure_{}_to_{}", p.join("_"), mir_type_suffix(ret))
        }
        MirType::Ptr => "Ptr".to_string(),
        MirType::Never => "Never".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snow_typeck::ty::Ty;
    use snow_typeck::TypeRegistry;

    fn empty_registry() -> TypeRegistry {
        TypeRegistry {
            struct_defs: Default::default(),
            type_aliases: Default::default(),
            sum_type_defs: Default::default(),
        }
    }

    #[test]
    fn resolve_int() {
        let reg = empty_registry();
        assert_eq!(resolve_type(&Ty::int(), &reg, false), MirType::Int);
    }

    #[test]
    fn resolve_float() {
        let reg = empty_registry();
        assert_eq!(resolve_type(&Ty::float(), &reg, false), MirType::Float);
    }

    #[test]
    fn resolve_bool() {
        let reg = empty_registry();
        assert_eq!(resolve_type(&Ty::bool(), &reg, false), MirType::Bool);
    }

    #[test]
    fn resolve_string() {
        let reg = empty_registry();
        assert_eq!(resolve_type(&Ty::string(), &reg, false), MirType::String);
    }

    #[test]
    fn resolve_unit_tuple() {
        let reg = empty_registry();
        assert_eq!(resolve_type(&Ty::Tuple(vec![]), &reg, false), MirType::Unit);
    }

    #[test]
    fn resolve_tuple() {
        let reg = empty_registry();
        let ty = Ty::Tuple(vec![Ty::int(), Ty::string()]);
        assert_eq!(
            resolve_type(&ty, &reg, false),
            MirType::Tuple(vec![MirType::Int, MirType::String])
        );
    }

    #[test]
    fn resolve_fn_ptr() {
        let reg = empty_registry();
        let ty = Ty::fun(vec![Ty::int()], Ty::string());
        assert_eq!(
            resolve_type(&ty, &reg, false),
            MirType::FnPtr(vec![MirType::Int], Box::new(MirType::String))
        );
    }

    #[test]
    fn resolve_closure() {
        let reg = empty_registry();
        let ty = Ty::fun(vec![Ty::int()], Ty::string());
        assert_eq!(
            resolve_type(&ty, &reg, true),
            MirType::Closure(vec![MirType::Int], Box::new(MirType::String))
        );
    }

    #[test]
    fn resolve_never() {
        let reg = empty_registry();
        assert_eq!(resolve_type(&Ty::Never, &reg, false), MirType::Never);
    }

    #[test]
    fn resolve_option_sum_type() {
        use snow_typeck::{SumTypeDefInfo, VariantInfo};

        let mut reg = empty_registry();
        reg.sum_type_defs.insert(
            "Option".to_string(),
            SumTypeDefInfo {
                name: "Option".to_string(),
                generic_params: vec!["T".to_string()],
                variants: vec![
                    VariantInfo {
                        name: "Some".to_string(),
                        fields: vec![],
                    },
                    VariantInfo {
                        name: "None".to_string(),
                        fields: vec![],
                    },
                ],
            },
        );
        let ty = Ty::option(Ty::int());
        assert_eq!(
            resolve_type(&ty, &reg, false),
            MirType::SumType("Option_Int".to_string())
        );
    }

    #[test]
    fn resolve_struct_no_args() {
        use snow_typeck::StructDefInfo;

        let mut reg = empty_registry();
        reg.struct_defs.insert(
            "Point".to_string(),
            StructDefInfo {
                name: "Point".to_string(),
                generic_params: vec![],
                fields: vec![],
            },
        );

        // Ty::App(Con("Point"), []) resolves to Struct("Point")
        let ty = Ty::struct_ty("Point", vec![]);
        assert_eq!(
            resolve_type(&ty, &reg, false),
            MirType::Struct("Point".to_string())
        );
    }

    #[test]
    fn mangle_generic_type() {
        let reg = empty_registry();
        let name = mangle_type_name("Result", &[Ty::int(), Ty::string()], &reg);
        assert_eq!(name, "Result_Int_String");
    }

    #[test]
    #[should_panic(expected = "Unresolved type variable")]
    fn resolve_var_panics() {
        use snow_typeck::ty::TyVar;
        let reg = empty_registry();
        resolve_type(&Ty::Var(TyVar(0)), &reg, false);
    }
}
