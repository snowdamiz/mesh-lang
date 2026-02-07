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
/// | String         | ptr (opaque pointer to SnowString)|
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
            } else {
                // Fallback: just tag byte
                context.struct_type(&[context.i8_type().into()], false).into()
            }
        }
        MirType::FnPtr(_, _) => context.ptr_type(inkwell::AddressSpace::default()).into(),
        MirType::Closure(_, _) => closure_type(context).into(),
        MirType::Ptr => context.ptr_type(inkwell::AddressSpace::default()).into(),
        MirType::Never => context.i8_type().into(),
        // Actor PID is an opaque pointer to the process struct.
        MirType::Pid(_) => context.ptr_type(inkwell::AddressSpace::default()).into(),
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
/// Layout: `{ i8 tag, [max_payload_size x i8] payload }`
///
/// The tag is a single byte (supports up to 256 variants).
/// The payload is a byte array sized to the largest variant.
pub fn create_sum_type_layout<'ctx>(
    context: &'ctx Context,
    sum_type: &MirSumTypeDef,
    struct_types: &FxHashMap<String, StructType<'ctx>>,
    sum_type_layouts: &FxHashMap<String, StructType<'ctx>>,
) -> StructType<'ctx> {
    let tag_type = context.i8_type();

    // Calculate max payload size across all variants.
    let target_data = inkwell::targets::TargetData::create(""); // temp for size queries
    let max_payload_size = sum_type
        .variants
        .iter()
        .map(|v| {
            if v.fields.is_empty() {
                0u64
            } else {
                let field_types: Vec<BasicTypeEnum<'ctx>> = v
                    .fields
                    .iter()
                    .map(|f| llvm_type(context, f, struct_types, sum_type_layouts))
                    .collect();
                let variant_struct = context.struct_type(&field_types, false);
                target_data.get_store_size(&variant_struct)
            }
        })
        .max()
        .unwrap_or(0);

    if max_payload_size == 0 {
        // Enum with no payloads (all nullary variants) -- just the tag.
        context.struct_type(&[tag_type.into()], false)
    } else {
        let payload_type = context.i8_type().array_type(max_payload_size as u32);
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

        let layout = create_sum_type_layout(&context, &sum_def, &structs, &sums);
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

        let layout = create_sum_type_layout(&context, &sum_def, &structs, &sums);
        // { i8 tag, [N x i8] payload }
        assert_eq!(layout.count_fields(), 2);
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
