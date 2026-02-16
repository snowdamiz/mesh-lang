//! MirType to LLVM type mapping.
//!
//! Converts MIR types (concrete, monomorphized) to their LLVM IR representations
//! using Inkwell. Handles scalar types, composite types (tuples, structs),
//! tagged union layouts for sum types, closure pairs, and function pointers.

use inkwell::context::Context;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, StructType};
use rustc_hash::FxHashMap;

use crate::mir::{MirSumTypeDef, MirType};

/// Convert a `MirType` to its LLVM `BasicTypeEnum` representation.
///
/// # Type mapping
///
/// | MirType        | LLVM Type                         |
/// |----------------|-----------------------------------|
/// | Int            | i64                               |
/// | Float          | f64                               |
/// | Bool           | i1                                |
/// | String         | ptr (opaque pointer to MeshString)|
/// | Unit           | {} (empty struct)                 |
/// | Tuple(elems)   | { elem0, elem1, ... }             |
/// | Struct(name)   | named struct from cache           |
/// | SumType(name)  | tagged union from cache           |
/// | FnPtr(..)      | ptr (function pointer)            |
/// | Closure(..)    | { ptr, ptr } (fn_ptr + env_ptr)   |
/// | Ptr            | ptr (opaque pointer)              |
/// | Never          | i8 (unreachable, placeholder)     |
pub fn llvm_type<'ctx>(
    context: &'ctx Context,
    ty: &MirType,
    struct_types: &FxHashMap<String, StructType<'ctx>>,
    sum_type_layouts: &FxHashMap<String, StructType<'ctx>>,
) -> BasicTypeEnum<'ctx> {
    match ty {
        MirType::Int => context.i64_type().into(),
        MirType::Float => context.f64_type().into(),
        MirType::Bool => context.bool_type().into(),
        MirType::String => context.ptr_type(inkwell::AddressSpace::default()).into(),
        MirType::Unit => context.struct_type(&[], false).into(),
        MirType::Tuple(elems) => {
            let field_types: Vec<BasicTypeEnum<'ctx>> = elems
                .iter()
                .map(|e| llvm_type(context, e, struct_types, sum_type_layouts))
                .collect();
            let field_refs: Vec<BasicTypeEnum<'ctx>> = field_types;
            context.struct_type(&field_refs, false).into()
        }
        MirType::Struct(name) => {
            if let Some(st) = struct_types.get(name) {
                (*st).into()
            } else {
                // Fallback: return opaque struct
                context
                    .opaque_struct_type(name)
                    .as_basic_type_enum()
            }
        }
        MirType::SumType(name) => {
            if let Some(st) = sum_type_layouts.get(name) {
                (*st).into()
            } else if let Some(base) = name.split('_').next() {
                // Monomorphized type (e.g., Result_String_String -> Result)
                if let Some(st) = sum_type_layouts.get(base) {
                    (*st).into()
                } else {
                    context.struct_type(&[context.i8_type().into()], false).into()
                }
            } else {
                // Fallback: just tag byte
                context.struct_type(&[context.i8_type().into()], false).into()
            }
        }
        MirType::FnPtr(_, _) => context.ptr_type(inkwell::AddressSpace::default()).into(),
        MirType::Closure(_, _) => closure_type(context).into(),
        MirType::Ptr => context.ptr_type(inkwell::AddressSpace::default()).into(),
        MirType::Never => context.i8_type().into(),
        // Actor PID is i64 at the LLVM level (u64 at runtime, type-safety is compile-time only).
        MirType::Pid(_) => context.i64_type().into(),
    }
}

/// The LLVM type for a closure value: `{ ptr, ptr }` (fn_ptr, env_ptr).
pub fn closure_type(context: &Context) -> StructType<'_> {
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    context.struct_type(&[ptr_ty.into(), ptr_ty.into()], false)
}

/// Build an LLVM function type from MIR parameter and return types.
pub fn llvm_fn_type<'ctx>(
    context: &'ctx Context,
    params: &[MirType],
    return_type: &MirType,
    struct_types: &FxHashMap<String, StructType<'ctx>>,
    sum_type_layouts: &FxHashMap<String, StructType<'ctx>>,
) -> FunctionType<'ctx> {
    let ret = llvm_type(context, return_type, struct_types, sum_type_layouts);
    let param_types: Vec<BasicMetadataTypeEnum<'ctx>> = params
        .iter()
        .map(|p| llvm_type(context, p, struct_types, sum_type_layouts).into())
        .collect();
    ret.fn_type(&param_types, false)
}

/// Build an LLVM function type for a closure function.
///
/// Closure functions take an env_ptr (ptr) as the first parameter,
/// followed by the user-visible parameters.
pub fn llvm_closure_fn_type<'ctx>(
    context: &'ctx Context,
    params: &[MirType],
    return_type: &MirType,
    struct_types: &FxHashMap<String, StructType<'ctx>>,
    sum_type_layouts: &FxHashMap<String, StructType<'ctx>>,
) -> FunctionType<'ctx> {
    let ret = llvm_type(context, return_type, struct_types, sum_type_layouts);
    let ptr_ty: BasicMetadataTypeEnum<'ctx> =
        context.ptr_type(inkwell::AddressSpace::default()).into();
    let mut param_types: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty];
    for p in params {
        param_types.push(llvm_type(context, p, struct_types, sum_type_layouts).into());
    }
    ret.fn_type(&param_types, false)
}

/// Create the tagged union layout for a sum type.
///
/// Layout: `{ i8 tag, payload_type }` where payload_type is sized to fit
/// the largest variant's fields.
///
/// For sum types with pointer-sized payloads (Result, Option from runtime),
/// the layout uses `{ i8, ptr }` which correctly aligns the pointer field.
/// For other sum types, uses `{ i8, [N x i8] }` as a byte array.
pub fn create_sum_type_layout<'ctx>(
    context: &'ctx Context,
    sum_type: &MirSumTypeDef,
    struct_types: &FxHashMap<String, StructType<'ctx>>,
    sum_type_layouts: &FxHashMap<String, StructType<'ctx>>,
    target_data: &inkwell::targets::TargetData,
) -> StructType<'ctx> {
    let tag_type = context.i8_type();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());

    // Check if all non-empty variants have exactly one pointer-sized field.
    // This is true for runtime sum types like Result and Option.
    let all_single_ptr = sum_type.variants.iter().all(|v| {
        v.fields.is_empty()
            || (v.fields.len() == 1 && matches!(v.fields[0], MirType::Ptr | MirType::String))
    });

    if all_single_ptr {
        let has_payload = sum_type.variants.iter().any(|v| !v.fields.is_empty());
        if has_payload {
            // Use { i8, ptr } -- matches runtime's #[repr(C)] { u8, *mut u8 } layout
            return context.struct_type(&[tag_type.into(), ptr_type.into()], false);
        } else {
            return context.struct_type(&[tag_type.into()], false);
        }
    }

    // Calculate max variant overlay size across all variants.
    // We use the full overlay type { i8 tag, field0, field1, ... } to account
    // for alignment padding between the tag and the first field.
    let max_overlay_size = sum_type
        .variants
        .iter()
        .map(|v| {
            if v.fields.is_empty() {
                // Just the tag byte
                1u64
            } else {
                let overlay = variant_struct_type(context, &v.fields, struct_types, sum_type_layouts);
                target_data.get_store_size(&overlay)
            }
        })
        .max()
        .unwrap_or(1);

    if max_overlay_size <= 1 {
        // Enum with no payloads (all nullary variants) -- just the tag.
        context.struct_type(&[tag_type.into()], false)
    } else {
        // Allocate enough bytes for the largest variant overlay.
        // Subtract 1 for the tag byte we add explicitly.
        let payload_bytes = max_overlay_size - 1;
        let payload_type = context.i8_type().array_type(payload_bytes as u32);
        context.struct_type(&[tag_type.into(), payload_type.into()], false)
    }
}

/// Create the variant overlay struct type for a specific variant.
///
/// Layout: `{ i8 tag, field0_type, field1_type, ... }`
///
/// This is used when constructing or destructuring a specific variant --
/// the payload bytes are reinterpreted through this struct type via GEP.
pub fn variant_struct_type<'ctx>(
    context: &'ctx Context,
    field_types: &[MirType],
    struct_types: &FxHashMap<String, StructType<'ctx>>,
    sum_type_layouts: &FxHashMap<String, StructType<'ctx>>,
) -> StructType<'ctx> {
    let tag_type = context.i8_type();
    let mut fields: Vec<BasicTypeEnum<'ctx>> = vec![tag_type.into()];
    for f in field_types {
        fields.push(llvm_type(context, f, struct_types, sum_type_layouts));
    }
    context.struct_type(&fields, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{MirSumTypeDef, MirType, MirVariantDef};
    use inkwell::context::Context;

    #[test]
    fn test_scalar_types() {
        let context = Context::create();
        let structs = FxHashMap::default();
        let sums = FxHashMap::default();

        let int_ty = llvm_type(&context, &MirType::Int, &structs, &sums);
        assert!(int_ty.is_int_type());

        let float_ty = llvm_type(&context, &MirType::Float, &structs, &sums);
        assert!(float_ty.is_float_type());

        let bool_ty = llvm_type(&context, &MirType::Bool, &structs, &sums);
        assert!(bool_ty.is_int_type());

        let str_ty = llvm_type(&context, &MirType::String, &structs, &sums);
        assert!(str_ty.is_pointer_type());

        let unit_ty = llvm_type(&context, &MirType::Unit, &structs, &sums);
        assert!(unit_ty.is_struct_type());
    }

    #[test]
    fn test_tuple_type() {
        let context = Context::create();
        let structs = FxHashMap::default();
        let sums = FxHashMap::default();

        let tuple_ty = llvm_type(
            &context,
            &MirType::Tuple(vec![MirType::Int, MirType::Float]),
            &structs,
            &sums,
        );
        assert!(tuple_ty.is_struct_type());
        let st = tuple_ty.into_struct_type();
        assert_eq!(st.count_fields(), 2);
    }

    #[test]
    fn test_closure_type_shape() {
        let context = Context::create();
        let ct = closure_type(&context);
        assert_eq!(ct.count_fields(), 2);
        // Both fields should be pointer types
        assert!(ct.get_field_type_at_index(0).unwrap().is_pointer_type());
        assert!(ct.get_field_type_at_index(1).unwrap().is_pointer_type());
    }

    #[test]
    fn test_sum_type_layout_nullary() {
        let context = Context::create();
        let structs = FxHashMap::default();
        let sums = FxHashMap::default();

        let sum_def = MirSumTypeDef {
            name: "Color".to_string(),
            variants: vec![
                MirVariantDef {
                    name: "Red".to_string(),
                    fields: vec![],
                    tag: 0,
                },
                MirVariantDef {
                    name: "Green".to_string(),
                    fields: vec![],
                    tag: 1,
                },
            ],
        };

        let td = inkwell::targets::TargetData::create("");
        let layout = create_sum_type_layout(&context, &sum_def, &structs, &sums, &td);
        // Nullary variants: just { i8 }
        assert_eq!(layout.count_fields(), 1);
    }

    #[test]
    fn test_sum_type_layout_with_payload() {
        let context = Context::create();
        let structs = FxHashMap::default();
        let sums = FxHashMap::default();

        let sum_def = MirSumTypeDef {
            name: "Option".to_string(),
            variants: vec![
                MirVariantDef {
                    name: "Some".to_string(),
                    fields: vec![MirType::Int],
                    tag: 0,
                },
                MirVariantDef {
                    name: "None".to_string(),
                    fields: vec![],
                    tag: 1,
                },
            ],
        };

        let td = inkwell::targets::TargetData::create("");
        let layout = create_sum_type_layout(&context, &sum_def, &structs, &sums, &td);
        // { i8 tag, [N x i8] payload }
        assert_eq!(layout.count_fields(), 2);
    }

    #[test]
    fn test_pid_type_is_i64() {
        let context = Context::create();
        let structs = FxHashMap::default();
        let sums = FxHashMap::default();

        // Untyped Pid -> i64
        let pid_ty = llvm_type(&context, &MirType::Pid(None), &structs, &sums);
        assert!(pid_ty.is_int_type(), "Pid should map to i64");
        assert_eq!(pid_ty.into_int_type().get_bit_width(), 64);

        // Typed Pid<Int> -> also i64
        let pid_int_ty = llvm_type(
            &context,
            &MirType::Pid(Some(Box::new(MirType::Int))),
            &structs,
            &sums,
        );
        assert!(pid_int_ty.is_int_type(), "Pid<Int> should also map to i64");
        assert_eq!(pid_int_ty.into_int_type().get_bit_width(), 64);
    }

    #[test]
    fn test_fn_type() {
        let context = Context::create();
        let structs = FxHashMap::default();
        let sums = FxHashMap::default();

        let fn_ty = llvm_fn_type(
            &context,
            &[MirType::Int, MirType::Int],
            &MirType::Int,
            &structs,
            &sums,
        );
        assert_eq!(fn_ty.count_param_types(), 2);
    }
}
