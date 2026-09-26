//! Decision tree to LLVM branch/switch translation.
//!
//! Translates compiled `DecisionTree` nodes from the pattern compilation
//! phase into LLVM basic blocks with switch instructions, conditional
//! branches, and variable bindings.
//!
//! ## Strategy
//!
//! - `Leaf`: Bind variables from AccessPath, codegen arm body, store result,
//!   branch to merge block
//! - `Switch`: Load tag from scrutinee, emit LLVM switch instruction,
//!   recurse for each case
//! - `Test`: Load scrutinee value, compare with literal, conditional branch,
//!   recurse for success/failure
//! - `Guard`: Codegen guard expression, conditional branch, recurse
//! - `Fail`: Emit mesh_panic + unreachable

use inkwell::basic_block::BasicBlock;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::IntPredicate;

use super::intrinsics::get_intrinsic;
use super::types::variant_struct_type;
use super::{CodeGen, SavedLocal};
use crate::mir::{MirLiteral, MirMatchArm, MirType};
use crate::pattern::{AccessPath, DecisionTree};

/// Where a match's decision tree reads its scrutinee and leaves its result.
#[derive(Clone, Copy)]
pub(crate) struct MatchTarget<'a, 'ctx> {
    /// The stack slot holding the scrutinee, which access paths start from.
    pub(crate) scrutinee: PointerValue<'ctx>,
    pub(crate) scrutinee_ty: &'a MirType,
    pub(crate) arms: &'a [MirMatchArm],
    /// The type every arm evaluates to, and the slot an arm stores it in.
    pub(crate) result_ty: &'a MirType,
    pub(crate) result: PointerValue<'ctx>,
    /// Where an arm branches once it is done.
    pub(crate) merge_bb: BasicBlock<'ctx>,
}

type Binding = (String, MirType, AccessPath);

impl<'ctx> CodeGen<'ctx> {
    /// Generate LLVM IR for a decision tree, compiled from the match arms of
    /// `target`, which controls which arm body runs.
    pub(crate) fn codegen_decision_tree(
        &mut self,
        tree: &DecisionTree,
        target: MatchTarget<'_, 'ctx>,
    ) -> Result<(), String> {
        match tree {
            DecisionTree::Leaf {
                arm_index,
                bindings,
            } => self.codegen_leaf(*arm_index, bindings, target),
            DecisionTree::Switch {
                scrutinee_path,
                cases,
                default,
            } => self.codegen_switch(scrutinee_path, cases, default.as_deref(), target),
            DecisionTree::Test {
                scrutinee_path,
                value,
                success,
                failure,
            } => self.codegen_test(scrutinee_path, value, success, failure, target),
            DecisionTree::Guard {
                guard_expr,
                arm_index,
                bindings,
                failure,
            } => self.codegen_guard(guard_expr, *arm_index, bindings, failure, target),
            DecisionTree::ListDecons {
                scrutinee_path,
                non_empty,
                empty,
                ..
            } => self.codegen_list_decons(scrutinee_path, non_empty, empty, target),
            DecisionTree::Fail {
                message,
                file,
                line,
            } => self.codegen_panic(message, file, *line).map(drop),
        }
    }

    /// Branch to `success_bb` when `cond` holds and to `failure_bb` when it
    /// does not, then generate `success` in the one and `failure` in the other.
    fn codegen_branches(
        &mut self,
        cond: IntValue<'ctx>,
        (success_bb, success): (BasicBlock<'ctx>, &DecisionTree),
        (failure_bb, failure): (BasicBlock<'ctx>, &DecisionTree),
        target: MatchTarget<'_, 'ctx>,
    ) -> Result<(), String> {
        self.builder
            .build_conditional_branch(cond, success_bb, failure_bb)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(success_bb);
        self.codegen_decision_tree(success, target)?;
        self.builder.position_at_end(failure_bb);
        self.codegen_decision_tree(failure, target)
    }

    // ── Leaf node ────────────────────────────────────────────────────

    fn codegen_leaf(
        &mut self,
        arm_index: usize,
        bindings: &[Binding],
        target: MatchTarget<'_, 'ctx>,
    ) -> Result<(), String> {
        // Bind variables from access paths. A binding shadows an outer name
        // only for the arm body.
        let saved = self.bind_pattern_values(bindings, target)?;
        let body_val = self.codegen_expr(&target.arms[arm_index].body);
        self.restore_locals(saved);
        let body_val = body_val?;

        // Store result and branch to merge (only if not already terminated by
        // return/panic). The ? operator desugaring generates match arms with
        // MirExpr::Return for early-return paths -- these emit a `ret`
        // instruction that terminates the block, so we must skip the store.
        if self
            .builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none()
        {
            let body_val = self.coerce_value_to_type(body_val, self.llvm_type(target.result_ty))?;
            self.builder
                .build_store(target.result, body_val)
                .map_err(|e| e.to_string())?;
            self.builder
                .build_unconditional_branch(target.merge_bb)
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    /// Bind each pattern variable to the value at its access path. Returns
    /// what the names meant before, for `restore_locals`.
    fn bind_pattern_values(
        &mut self,
        bindings: &[Binding],
        target: MatchTarget<'_, 'ctx>,
    ) -> Result<Vec<SavedLocal<'ctx>>, String> {
        bindings
            .iter()
            .map(|(name, ty, path)| {
                let val = self.navigate_access_path(target.scrutinee, target.scrutinee_ty, path)?;
                self.bind_local(name, ty, val)
            })
            .collect()
    }

    // ── Switch node ──────────────────────────────────────────────────

    fn codegen_switch(
        &mut self,
        scrutinee_path: &AccessPath,
        cases: &[(crate::pattern::ConstructorTag, DecisionTree)],
        default: Option<&DecisionTree>,
        target: MatchTarget<'_, 'ctx>,
    ) -> Result<(), String> {
        let fn_val = self.current_function();

        // Every sum type layout starts with its i8 tag.
        let switch_ptr =
            self.navigate_access_path_ptr(target.scrutinee, target.scrutinee_ty, scrutinee_path)?;
        let tag_val = self
            .builder
            .build_load(self.context.i8_type(), switch_ptr, "tag")
            .map_err(|e| e.to_string())?
            .into_int_value();

        // Create blocks for each case
        let default_bb = self.context.append_basic_block(fn_val, "switch_default");
        let case_bbs: Vec<BasicBlock<'ctx>> = cases
            .iter()
            .map(|(tag, _)| {
                self.context
                    .append_basic_block(fn_val, &format!("case_{}", tag.variant_name))
            })
            .collect();

        // Build switch instruction with all cases
        let switch_cases: Vec<(inkwell::values::IntValue<'ctx>, BasicBlock<'ctx>)> = cases
            .iter()
            .enumerate()
            .map(|(i, (tag, _))| {
                let case_val = self.context.i8_type().const_int(tag.tag as u64, false);
                (case_val, case_bbs[i])
            })
            .collect();

        self.builder
            .build_switch(tag_val, default_bb, &switch_cases)
            .map_err(|e| e.to_string())?;

        // Generate code for each case
        for (i, (_, subtree)) in cases.iter().enumerate() {
            self.builder.position_at_end(case_bbs[i]);
            self.codegen_decision_tree(subtree, target)?;
        }

        // Generate default case
        self.builder.position_at_end(default_bb);
        match default {
            Some(default_tree) => self.codegen_decision_tree(default_tree, target),
            // Default: unreachable (exhaustive match guaranteed by type checker)
            None => {
                let fn_name = self.current_function_name();
                self.codegen_panic("non-exhaustive match in switch", &fn_name, 0)
                    .map(drop)
            }
        }
    }

    // ── Test node ────────────────────────────────────────────────────

    fn codegen_test(
        &mut self,
        scrutinee_path: &AccessPath,
        value: &MirLiteral,
        success: &DecisionTree,
        failure: &DecisionTree,
        target: MatchTarget<'_, 'ctx>,
    ) -> Result<(), String> {
        let fn_val = self.current_function();

        // The value at the access path. A literal inside a generic payload is
        // read through its box: the path has the literal's own type.
        let test_val =
            self.navigate_access_path(target.scrutinee, target.scrutinee_ty, scrutinee_path)?;

        // Compare with the literal
        let cond = match value {
            MirLiteral::Int(n) => {
                let lit_val = self.context.i64_type().const_int(*n as u64, true);
                self.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        test_val.into_int_value(),
                        lit_val,
                        "test_eq",
                    )
                    .map_err(|e| e.to_string())?
            }
            MirLiteral::Float(f) => {
                let lit_val = self.context.f64_type().const_float(*f);
                self.builder
                    .build_float_compare(
                        inkwell::FloatPredicate::OEQ,
                        test_val.into_float_value(),
                        lit_val,
                        "test_feq",
                    )
                    .map_err(|e| e.to_string())?
            }
            MirLiteral::Bool(b) => {
                let lit_val = self
                    .context
                    .bool_type()
                    .const_int(if *b { 1 } else { 0 }, false);
                self.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        test_val.into_int_value(),
                        lit_val,
                        "test_beq",
                    )
                    .map_err(|e| e.to_string())?
            }
            MirLiteral::String(s) => {
                // Create a MeshString for the pattern literal
                let pattern_str = self.codegen_string_lit(s)?;

                // Call mesh_string_eq(scrutinee, pattern)
                let eq_fn = get_intrinsic(&self.module, "mesh_string_eq");
                let result = self
                    .builder
                    .build_call(eq_fn, &[test_val.into(), pattern_str.into()], "str_eq")
                    .map_err(|e| e.to_string())?;
                let i8_result = result
                    .try_as_basic_value()
                    .basic()
                    .ok_or("mesh_string_eq returned void")?
                    .into_int_value();

                // Convert i8 result to i1 for branch condition
                let zero = self.context.i8_type().const_int(0, false);
                self.builder
                    .build_int_compare(IntPredicate::NE, i8_result, zero, "str_eq_bool")
                    .map_err(|e| e.to_string())?
            }
        };

        let success_bb = self.context.append_basic_block(fn_val, "test_success");
        let failure_bb = self.context.append_basic_block(fn_val, "test_failure");
        self.codegen_branches(cond, (success_bb, success), (failure_bb, failure), target)
    }

    // ── Guard node ───────────────────────────────────────────────────

    /// Evaluate a guard with the bindings of the arm it guards in scope, and
    /// run that arm when it holds.
    fn codegen_guard(
        &mut self,
        guard_expr: &crate::mir::MirExpr,
        arm_index: usize,
        bindings: &[Binding],
        failure: &DecisionTree,
        target: MatchTarget<'_, 'ctx>,
    ) -> Result<(), String> {
        let fn_val = self.current_function();

        let saved = self.bind_pattern_values(bindings, target)?;
        let guard_val = self.codegen_expr(guard_expr);
        self.restore_locals(saved);
        let guard_val = guard_val?.into_int_value();

        let success_bb = self.context.append_basic_block(fn_val, "guard_pass");
        let failure_bb = self.context.append_basic_block(fn_val, "guard_fail");
        self.builder
            .build_conditional_branch(guard_val, success_bb, failure_bb)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(success_bb);
        self.codegen_leaf(arm_index, bindings, target)?;
        self.builder.position_at_end(failure_bb);
        self.codegen_decision_tree(failure, target)
    }

    // ── ListDecons node ──────────────────────────────────────────────

    fn codegen_list_decons(
        &mut self,
        scrutinee_path: &AccessPath,
        non_empty: &DecisionTree,
        empty: &DecisionTree,
        target: MatchTarget<'_, 'ctx>,
    ) -> Result<(), String> {
        let fn_val = self.current_function();

        // Load the list pointer at the access path.
        let list_val =
            self.navigate_access_path(target.scrutinee, target.scrutinee_ty, scrutinee_path)?;
        let list_ptr = list_val.into_pointer_value();

        // Call mesh_list_length(list) to check if non-empty.
        let length_fn = get_intrinsic(&self.module, "mesh_list_length");
        let length_result = self
            .builder
            .build_call(length_fn, &[list_ptr.into()], "list_len")
            .map_err(|e| e.to_string())?;
        let length_val = length_result
            .try_as_basic_value()
            .basic()
            .ok_or("mesh_list_length returned void")?
            .into_int_value();

        // Compare length > 0.
        let zero = self.context.i64_type().const_int(0, false);
        let is_non_empty = self
            .builder
            .build_int_compare(IntPredicate::SGT, length_val, zero, "is_non_empty")
            .map_err(|e| e.to_string())?;

        let non_empty_bb = self.context.append_basic_block(fn_val, "list_non_empty");
        let empty_bb = self.context.append_basic_block(fn_val, "list_empty");
        self.codegen_branches(
            is_non_empty,
            (non_empty_bb, non_empty),
            (empty_bb, empty),
            target,
        )
    }

    // ── Access path navigation ───────────────────────────────────────

    /// Navigate an access path and return the loaded value.
    fn navigate_access_path(
        &mut self,
        scrutinee_alloca: PointerValue<'ctx>,
        scrutinee_ty: &MirType,
        path: &AccessPath,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.navigate_access_path_ptr(scrutinee_alloca, scrutinee_ty, path)?;
        // Tuple expressions use a runtime pointer, including control-flow results
        // and nested tuple fields. Do not load the semantic by-value tuple type.
        let llvm_ty = match path.ty(scrutinee_ty) {
            MirType::Tuple(_) => self
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .into(),
            path_ty => self.llvm_type(path_ty),
        };
        self.builder
            .build_load(llvm_ty, ptr, "path_val")
            .map_err(|e| e.to_string())
    }

    /// Navigate an access path and return a pointer to the value.
    fn navigate_access_path_ptr(
        &mut self,
        scrutinee_alloca: PointerValue<'ctx>,
        scrutinee_ty: &MirType,
        path: &AccessPath,
    ) -> Result<PointerValue<'ctx>, String> {
        match path {
            AccessPath::Root => Ok(scrutinee_alloca),

            AccessPath::Column(index, _) => self
                .builder
                .build_struct_gep(
                    self.llvm_type(scrutinee_ty).into_struct_type(),
                    scrutinee_alloca,
                    *index as u32,
                    "column",
                )
                .map_err(|e| e.to_string()),

            AccessPath::TupleField(parent, index, element_ty) => {
                // Tuples use the runtime layout `{ u64 len, u64 elements[] }`,
                // including when a generic constructor stores the tuple as Ptr.
                let tuple_ptr = self
                    .navigate_access_path(scrutinee_alloca, scrutinee_ty, parent)?
                    .into_pointer_value();
                let nth_fn = get_intrinsic(&self.module, "mesh_tuple_nth");
                let index = self.context.i64_type().const_int(*index as u64, false);
                let element = self
                    .builder
                    .build_call(nth_fn, &[tuple_ptr.into(), index.into()], "tuple_field")
                    .map_err(|e| e.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("mesh_tuple_nth returned void")?
                    .into_int_value();

                self.materialize_tuple_element_ptr(element, element_ty)
            }

            AccessPath::VariantField {
                parent,
                type_name,
                variant,
                index,
                ty: semantic_ty,
            } => {
                let parent_ptr =
                    self.navigate_access_path_ptr(scrutinee_alloca, scrutinee_ty, parent)?;
                let variant_def = self
                    .lookup_sum_type_def(type_name)
                    .and_then(|def| def.variants.iter().find(|v| v.name == *variant))
                    .ok_or_else(|| format!("Unknown variant '{type_name}.{variant}'"))?;
                let storage_ty = variant_def.fields[*index].clone();

                // Create variant overlay type { i8 tag, field0, field1, ... }
                let variant_ty = variant_struct_type(
                    self.context,
                    &variant_def.fields,
                    &self.struct_types,
                    &self.sum_type_layouts,
                );

                // GEP into the variant overlay (field 0 is tag, so field N is index+1)
                let field_ptr = self
                    .builder
                    .build_struct_gep(variant_ty, parent_ptr, (*index + 1) as u32, "variant_field")
                    .map_err(|e| e.to_string())?;
                // A tuple is itself a pointer, stored as the slot's word; any
                // other value, a pid included, is boxed.
                if matches!(storage_ty, MirType::Ptr | MirType::Struct(_))
                    && !matches!(
                        semantic_ty,
                        MirType::Ptr | MirType::String | MirType::Tuple(_)
                    )
                {
                    self.builder
                        .build_load(
                            self.context.ptr_type(inkwell::AddressSpace::default()),
                            field_ptr,
                            "boxed_variant_payload",
                        )
                        .map(|value| value.into_pointer_value())
                        .map_err(|error| error.to_string())
                } else {
                    Ok(field_ptr)
                }
            }

            AccessPath::StructField {
                parent,
                name,
                index,
                ..
            } => {
                let parent_ptr =
                    self.navigate_access_path_ptr(scrutinee_alloca, scrutinee_ty, parent)?;
                let struct_ty = self
                    .llvm_type(&MirType::Struct(name.clone()))
                    .into_struct_type();
                self.builder
                    .build_struct_gep(struct_ty, parent_ptr, *index as u32, "struct_field")
                    .map_err(|e| e.to_string())
            }

            AccessPath::ListHead(parent, elem_ty) => {
                // Load the list pointer, call mesh_list_head, store result in an alloca.
                let parent_val =
                    self.navigate_access_path(scrutinee_alloca, scrutinee_ty, parent)?;
                let list_ptr = parent_val.into_pointer_value();

                let head_fn = get_intrinsic(&self.module, "mesh_list_head");
                let head_result = self
                    .builder
                    .build_call(head_fn, &[list_ptr.into()], "list_head")
                    .map_err(|e| e.to_string())?;
                let head_i64 = head_result
                    .try_as_basic_value()
                    .basic()
                    .ok_or("mesh_list_head returned void")?
                    .into_int_value();

                // Convert u64 -> the element type.
                let converted = self.convert_from_list_element(head_i64, elem_ty)?;

                // Store in an alloca so we can return a pointer.
                let alloca = self
                    .builder
                    .build_alloca(self.llvm_type(elem_ty), "list_head_alloca")
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_store(alloca, converted)
                    .map_err(|e| e.to_string())?;
                Ok(alloca)
            }

            AccessPath::ListTail(parent) => {
                // Load the list pointer, call mesh_list_tail, store result in an alloca.
                let parent_val =
                    self.navigate_access_path(scrutinee_alloca, scrutinee_ty, parent)?;
                let list_ptr = parent_val.into_pointer_value();

                let tail_fn = get_intrinsic(&self.module, "mesh_list_tail");
                let tail_result = self
                    .builder
                    .build_call(tail_fn, &[list_ptr.into()], "list_tail")
                    .map_err(|e| e.to_string())?;
                let tail_ptr = tail_result
                    .try_as_basic_value()
                    .basic()
                    .ok_or("mesh_list_tail returned void")?
                    .into_pointer_value();

                // Store in an alloca so we can return a pointer.
                let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                let alloca = self
                    .builder
                    .build_alloca(ptr_ty, "list_tail_alloca")
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_store(alloca, tail_ptr)
                    .map_err(|e| e.to_string())?;
                Ok(alloca)
            }
        }
    }

    /// Materialize a uniformly stored tuple element as an addressable value.
    ///
    /// `mesh_tuple_nth` returns the raw u64 slot. One-slot values can be loaded
    /// from a temporary u64 alloca; larger aggregates were boxed by
    /// `codegen_make_tuple`, so their slot is already an address.
    pub(super) fn materialize_tuple_element_ptr(
        &self,
        value: IntValue<'ctx>,
        element_ty: &MirType,
    ) -> Result<PointerValue<'ctx>, String> {
        if matches!(
            element_ty,
            MirType::Struct(_) | MirType::SumType(_) | MirType::Closure(_, _)
        ) {
            let aggregate_ty = self.llvm_type(element_ty).into_struct_type();
            let size = self
                .target_machine
                .get_target_data()
                .get_store_size(&aggregate_ty);
            if size > 8 {
                return self
                    .builder
                    .build_int_to_ptr(
                        value,
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        "tuple_boxed_element",
                    )
                    .map_err(|e| e.to_string());
            }
        }

        let alloca = self
            .builder
            .build_alloca(self.context.i64_type(), "tuple_element")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(alloca, value)
            .map_err(|e| e.to_string())?;
        Ok(alloca)
    }
}
