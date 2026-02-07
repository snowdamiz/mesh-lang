//! MIR expression to LLVM IR translation.
//!
//! Implements `codegen_expr` which translates each MIR expression variant
//! into corresponding LLVM IR instructions using the alloca+mem2reg pattern
//! for control flow merges.

use inkwell::types::BasicType;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum};
use inkwell::IntPredicate;

use super::intrinsics::get_intrinsic;
use super::types::{closure_type, variant_struct_type};
use super::CodeGen;
use crate::mir::{BinOp, MirChildSpec, MirExpr, MirMatchArm, MirPattern, MirType, UnaryOp};
use crate::pattern::compile::compile_match;

impl<'ctx> CodeGen<'ctx> {
    /// Generate LLVM IR for a MIR expression.
    ///
    /// Returns the LLVM value representing the expression result.
    pub(crate) fn codegen_expr(
        &mut self,
        expr: &MirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match expr {
            MirExpr::IntLit(val, _) => {
                Ok(self.context.i64_type().const_int(*val as u64, true).into())
            }

            MirExpr::FloatLit(val, _) => {
                Ok(self.context.f64_type().const_float(*val).into())
            }

            MirExpr::BoolLit(val, _) => {
                Ok(self
                    .context
                    .bool_type()
                    .const_int(if *val { 1 } else { 0 }, false)
                    .into())
            }

            MirExpr::StringLit(s, _) => self.codegen_string_lit(s),

            MirExpr::Var(name, ty) => self.codegen_var(name, ty),

            MirExpr::BinOp { op, lhs, rhs, ty } => self.codegen_binop(op, lhs, rhs, ty),

            MirExpr::UnaryOp { op, operand, ty } => self.codegen_unaryop(op, operand, ty),

            MirExpr::Call { func, args, ty } => self.codegen_call(func, args, ty),

            MirExpr::ClosureCall { closure, args, ty } => {
                self.codegen_closure_call(closure, args, ty)
            }

            MirExpr::If {
                cond,
                then_body,
                else_body,
                ty,
            } => self.codegen_if(cond, then_body, else_body, ty),

            MirExpr::Let {
                name,
                ty,
                value,
                body,
            } => self.codegen_let(name, ty, value, body),

            MirExpr::Block(exprs, _ty) => self.codegen_block(exprs),

            MirExpr::Match {
                scrutinee,
                arms,
                ty,
            } => self.codegen_match(scrutinee, arms, ty),

            MirExpr::StructLit {
                name,
                fields,
                ty: _,
            } => self.codegen_struct_lit(name, fields),

            MirExpr::FieldAccess { object, field, ty } => {
                self.codegen_field_access(object, field, ty)
            }

            MirExpr::ConstructVariant {
                type_name,
                variant,
                fields,
                ty: _,
            } => self.codegen_construct_variant(type_name, variant, fields),

            MirExpr::MakeClosure {
                fn_name,
                captures,
                ty: _,
            } => self.codegen_make_closure(fn_name, captures),

            MirExpr::Return(inner) => self.codegen_return(inner),

            MirExpr::Panic {
                message,
                file,
                line,
            } => self.codegen_panic(message, file, *line),

            MirExpr::Unit => Ok(self.context.struct_type(&[], false).const_zero().into()),

            // Actor primitives
            MirExpr::ActorSpawn {
                func,
                args,
                priority,
                terminate_callback,
                ty: _,
            } => self.codegen_actor_spawn(func, args, *priority, terminate_callback.as_deref()),

            MirExpr::ActorSend {
                target,
                message,
                ty: _,
            } => self.codegen_actor_send(target, message),

            MirExpr::ActorReceive {
                arms,
                timeout_ms,
                timeout_body: _,
                ty,
            } => self.codegen_actor_receive(arms, timeout_ms.as_deref(), ty),

            MirExpr::ActorSelf { ty: _ } => self.codegen_actor_self(),

            MirExpr::ActorLink { target, ty: _ } => self.codegen_actor_link(target),

            MirExpr::SupervisorStart {
                name,
                strategy,
                max_restarts,
                max_seconds,
                children,
                ty: _,
            } => self.codegen_supervisor_start(name, *strategy, *max_restarts, *max_seconds, children),
        }
    }

    // ── String literals ──────────────────────────────────────────────

    fn codegen_string_lit(&mut self, s: &str) -> Result<BasicValueEnum<'ctx>, String> {
        // Create a global constant for the string data
        let str_val = self.context.const_string(s.as_bytes(), false);
        let global = self.module.add_global(str_val.get_type(), None, ".str");
        global.set_initializer(&str_val);
        global.set_constant(true);
        global.set_unnamed_addr(true);

        // Call snow_string_new(data_ptr, len)
        let data_ptr = global.as_pointer_value();
        let len = self
            .context
            .i64_type()
            .const_int(s.len() as u64, false);

        let string_new = get_intrinsic(&self.module, "snow_string_new");
        let result = self
            .builder
            .build_call(
                string_new,
                &[data_ptr.into(), len.into()],
                "str",
            )
            .map_err(|e| e.to_string())?;

        result
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "snow_string_new returned void".to_string())
    }

    // ── Variable reference ───────────────────────────────────────────

    fn codegen_var(
        &mut self,
        name: &str,
        ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Check if it's a known function reference (for passing as fn ptr)
        if let Some(fn_val) = self.functions.get(name) {
            return Ok(fn_val.as_global_value().as_pointer_value().into());
        }

        // Load from local variable alloca
        if let Some(alloca) = self.locals.get(name) {
            let alloca = *alloca;
            let llvm_ty = self.llvm_type(ty);
            let val = self
                .builder
                .build_load(llvm_ty, alloca, name)
                .map_err(|e| e.to_string())?;
            Ok(val)
        } else {
            Err(format!("Undefined variable '{}'", name))
        }
    }

    // ── Binary operations ────────────────────────────────────────────

    fn codegen_binop(
        &mut self,
        op: &BinOp,
        lhs: &MirExpr,
        rhs: &MirExpr,
        _ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Short-circuit for boolean And/Or
        match op {
            BinOp::And => return self.codegen_short_circuit_and(lhs, rhs),
            BinOp::Or => return self.codegen_short_circuit_or(lhs, rhs),
            _ => {}
        }

        let lhs_val = self.codegen_expr(lhs)?;
        let rhs_val = self.codegen_expr(rhs)?;

        let lhs_ty = lhs.ty();

        // String concat
        if matches!(op, BinOp::Concat) {
            return self.codegen_string_concat(lhs_val, rhs_val);
        }

        // String equality
        if matches!(lhs_ty, MirType::String) && matches!(op, BinOp::Eq | BinOp::NotEq) {
            return self.codegen_string_compare(op, lhs_val, rhs_val);
        }

        match lhs_ty {
            MirType::Int => self.codegen_int_binop(op, lhs_val, rhs_val),
            MirType::Float => self.codegen_float_binop(op, lhs_val, rhs_val),
            MirType::Bool => self.codegen_bool_binop(op, lhs_val, rhs_val),
            _ => Err(format!("Unsupported binop type: {:?}", lhs_ty)),
        }
    }

    fn codegen_int_binop(
        &mut self,
        op: &BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let l = lhs.into_int_value();
        let r = rhs.into_int_value();

        let result: BasicValueEnum<'ctx> = match op {
            BinOp::Add => self.builder.build_int_add(l, r, "add").map_err(|e| e.to_string())?.into(),
            BinOp::Sub => self.builder.build_int_sub(l, r, "sub").map_err(|e| e.to_string())?.into(),
            BinOp::Mul => self.builder.build_int_mul(l, r, "mul").map_err(|e| e.to_string())?.into(),
            BinOp::Div => self.builder.build_int_signed_div(l, r, "div").map_err(|e| e.to_string())?.into(),
            BinOp::Mod => self.builder.build_int_signed_rem(l, r, "mod").map_err(|e| e.to_string())?.into(),
            BinOp::Eq => self.builder.build_int_compare(IntPredicate::EQ, l, r, "eq").map_err(|e| e.to_string())?.into(),
            BinOp::NotEq => self.builder.build_int_compare(IntPredicate::NE, l, r, "ne").map_err(|e| e.to_string())?.into(),
            BinOp::Lt => self.builder.build_int_compare(IntPredicate::SLT, l, r, "lt").map_err(|e| e.to_string())?.into(),
            BinOp::Gt => self.builder.build_int_compare(IntPredicate::SGT, l, r, "gt").map_err(|e| e.to_string())?.into(),
            BinOp::LtEq => self.builder.build_int_compare(IntPredicate::SLE, l, r, "le").map_err(|e| e.to_string())?.into(),
            BinOp::GtEq => self.builder.build_int_compare(IntPredicate::SGE, l, r, "ge").map_err(|e| e.to_string())?.into(),
            _ => return Err(format!("Unsupported int binop: {:?}", op)),
        };
        Ok(result)
    }

    fn codegen_float_binop(
        &mut self,
        op: &BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let l = lhs.into_float_value();
        let r = rhs.into_float_value();

        let result: BasicValueEnum<'ctx> = match op {
            BinOp::Add => self.builder.build_float_add(l, r, "fadd").map_err(|e| e.to_string())?.into(),
            BinOp::Sub => self.builder.build_float_sub(l, r, "fsub").map_err(|e| e.to_string())?.into(),
            BinOp::Mul => self.builder.build_float_mul(l, r, "fmul").map_err(|e| e.to_string())?.into(),
            BinOp::Div => self.builder.build_float_div(l, r, "fdiv").map_err(|e| e.to_string())?.into(),
            BinOp::Mod => self.builder.build_float_rem(l, r, "fmod").map_err(|e| e.to_string())?.into(),
            BinOp::Eq => self.builder.build_float_compare(inkwell::FloatPredicate::OEQ, l, r, "feq").map_err(|e| e.to_string())?.into(),
            BinOp::NotEq => self.builder.build_float_compare(inkwell::FloatPredicate::ONE, l, r, "fne").map_err(|e| e.to_string())?.into(),
            BinOp::Lt => self.builder.build_float_compare(inkwell::FloatPredicate::OLT, l, r, "flt").map_err(|e| e.to_string())?.into(),
            BinOp::Gt => self.builder.build_float_compare(inkwell::FloatPredicate::OGT, l, r, "fgt").map_err(|e| e.to_string())?.into(),
            BinOp::LtEq => self.builder.build_float_compare(inkwell::FloatPredicate::OLE, l, r, "fle").map_err(|e| e.to_string())?.into(),
            BinOp::GtEq => self.builder.build_float_compare(inkwell::FloatPredicate::OGE, l, r, "fge").map_err(|e| e.to_string())?.into(),
            _ => return Err(format!("Unsupported float binop: {:?}", op)),
        };
        Ok(result)
    }

    fn codegen_bool_binop(
        &mut self,
        op: &BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let l = lhs.into_int_value();
        let r = rhs.into_int_value();

        let result: BasicValueEnum<'ctx> = match op {
            BinOp::Eq => self.builder.build_int_compare(IntPredicate::EQ, l, r, "beq").map_err(|e| e.to_string())?.into(),
            BinOp::NotEq => self.builder.build_int_compare(IntPredicate::NE, l, r, "bne").map_err(|e| e.to_string())?.into(),
            _ => return Err(format!("Unsupported bool binop: {:?}", op)),
        };
        Ok(result)
    }

    fn codegen_short_circuit_and(
        &mut self,
        lhs: &MirExpr,
        rhs: &MirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.current_function();
        let lhs_val = self.codegen_expr(lhs)?.into_int_value();

        let rhs_bb = self.context.append_basic_block(fn_val, "and_rhs");
        let merge_bb = self.context.append_basic_block(fn_val, "and_merge");

        self.builder
            .build_conditional_branch(lhs_val, rhs_bb, merge_bb)
            .map_err(|e| e.to_string())?;

        // RHS block
        self.builder.position_at_end(rhs_bb);
        let rhs_val = self.codegen_expr(rhs)?.into_int_value();
        let rhs_end_bb = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| e.to_string())?;

        // Merge block with phi
        self.builder.position_at_end(merge_bb);
        let phi = self
            .builder
            .build_phi(self.context.bool_type(), "and_result")
            .map_err(|e| e.to_string())?;

        let false_val = self.context.bool_type().const_int(0, false);
        let lhs_bb = fn_val
            .get_basic_blocks()
            .into_iter()
            .find(|bb| {
                // Find the block that branches to merge_bb but is not rhs_end_bb
                bb != &rhs_bb && bb != &merge_bb && bb != &rhs_end_bb
            });

        // Use the block where lhs was evaluated (could be entry or wherever)
        if let Some(lhs_end_bb) = lhs_bb {
            phi.add_incoming(&[(&false_val, lhs_end_bb), (&rhs_val, rhs_end_bb)]);
        } else {
            // Fallback: just use false
            phi.add_incoming(&[(&rhs_val, rhs_end_bb)]);
        }

        Ok(phi.as_basic_value())
    }

    fn codegen_short_circuit_or(
        &mut self,
        lhs: &MirExpr,
        rhs: &MirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.current_function();
        let lhs_val = self.codegen_expr(lhs)?.into_int_value();

        let rhs_bb = self.context.append_basic_block(fn_val, "or_rhs");
        let merge_bb = self.context.append_basic_block(fn_val, "or_merge");

        self.builder
            .build_conditional_branch(lhs_val, merge_bb, rhs_bb)
            .map_err(|e| e.to_string())?;

        let lhs_end_bb = self.builder.get_insert_block().unwrap();

        // RHS block
        self.builder.position_at_end(rhs_bb);
        let rhs_val = self.codegen_expr(rhs)?.into_int_value();
        let rhs_end_bb = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| e.to_string())?;

        // Merge
        self.builder.position_at_end(merge_bb);
        let phi = self
            .builder
            .build_phi(self.context.bool_type(), "or_result")
            .map_err(|e| e.to_string())?;

        let true_val = self.context.bool_type().const_int(1, false);
        phi.add_incoming(&[(&true_val, lhs_end_bb), (&rhs_val, rhs_end_bb)]);

        Ok(phi.as_basic_value())
    }

    fn codegen_string_concat(
        &mut self,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let concat_fn = get_intrinsic(&self.module, "snow_string_concat");
        let result = self
            .builder
            .build_call(concat_fn, &[lhs.into(), rhs.into()], "concat")
            .map_err(|e| e.to_string())?;
        result
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "snow_string_concat returned void".to_string())
    }

    fn codegen_string_compare(
        &mut self,
        op: &BinOp,
        _lhs: BasicValueEnum<'ctx>,
        _rhs: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // For now, string comparison compares pointers (identity).
        // A proper string comparison would need a runtime function.
        // This is a placeholder for Phase 5.
        let result = match op {
            BinOp::Eq => self.context.bool_type().const_int(0, false),
            BinOp::NotEq => self.context.bool_type().const_int(1, false),
            _ => return Err(format!("Unsupported string comparison: {:?}", op)),
        };
        Ok(result.into())
    }

    // ── Unary operations ─────────────────────────────────────────────

    fn codegen_unaryop(
        &mut self,
        op: &UnaryOp,
        operand: &MirExpr,
        _ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.codegen_expr(operand)?;
        match op {
            UnaryOp::Neg => {
                match operand.ty() {
                    MirType::Int => {
                        let int_val = val.into_int_value();
                        Ok(self
                            .builder
                            .build_int_neg(int_val, "neg")
                            .map_err(|e| e.to_string())?
                            .into())
                    }
                    MirType::Float => {
                        let float_val = val.into_float_value();
                        Ok(self
                            .builder
                            .build_float_neg(float_val, "fneg")
                            .map_err(|e| e.to_string())?
                            .into())
                    }
                    _ => Err(format!("Cannot negate type {:?}", operand.ty())),
                }
            }
            UnaryOp::Not => {
                let bool_val = val.into_int_value();
                Ok(self
                    .builder
                    .build_not(bool_val, "not")
                    .map_err(|e| e.to_string())?
                    .into())
            }
        }
    }

    // ── Function calls ───────────────────────────────────────────────

    fn codegen_call(
        &mut self,
        func: &MirExpr,
        args: &[MirExpr],
        ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Compile arguments
        let mut arg_vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
        for arg in args {
            let val = self.codegen_expr(arg)?;
            arg_vals.push(val.into());
        }

        // Check if it's a direct call to a known function
        if let MirExpr::Var(name, _) = func {
            if let Some(fn_val) = self.functions.get(name).copied() {
                let call = self
                    .builder
                    .build_call(fn_val, &arg_vals, "call")
                    .map_err(|e| e.to_string())?;

                // Insert reduction check after function call
                self.emit_reduction_check();

                if matches!(ty, MirType::Unit) {
                    return Ok(self.context.struct_type(&[], false).const_zero().into());
                }

                return call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "Function call returned void".to_string());
            }

            // Check if it's a runtime intrinsic (don't add reduction check for runtime calls)
            if let Some(fn_val) = self.module.get_function(name) {
                // Coerce argument types: runtime functions use i8 for booleans
                // but Snow's Bool is i1. Extend i1 -> i8 where needed.
                let mut coerced_args = arg_vals.clone();
                let param_types = fn_val.get_type().get_param_types();
                for (i, param_ty) in param_types.iter().enumerate() {
                    if i < coerced_args.len() {
                        if let BasicMetadataValueEnum::IntValue(arg_iv) = coerced_args[i] {
                            if let inkwell::types::BasicMetadataTypeEnum::IntType(param_it) = param_ty {
                                if arg_iv.get_type().get_bit_width() < param_it.get_bit_width() {
                                    let extended = self
                                        .builder
                                        .build_int_z_extend(arg_iv, *param_it, "zext_arg")
                                        .map_err(|e| e.to_string())?;
                                    coerced_args[i] = extended.into();
                                }
                            }
                        }
                    }
                }

                let call = self
                    .builder
                    .build_call(fn_val, &coerced_args, "call")
                    .map_err(|e| e.to_string())?;

                if matches!(ty, MirType::Unit) {
                    return Ok(self.context.struct_type(&[], false).const_zero().into());
                }

                let result = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "Function call returned void".to_string())?;

                // Runtime functions returning i8 for Bool values need truncation
                // to i1 to match Snow's Bool representation.
                if matches!(ty, MirType::Bool) {
                    if let BasicValueEnum::IntValue(iv) = result {
                        if iv.get_type().get_bit_width() == 8 {
                            let i1_val = self
                                .builder
                                .build_int_truncate(iv, self.context.bool_type(), "to_bool")
                                .map_err(|e| e.to_string())?;
                            return Ok(i1_val.into());
                        }
                    }
                }

                return Ok(result);
            }
        }

        // Indirect call through a function pointer or closure
        let fn_ptr = self.codegen_expr(func)?;
        let ret_ty = self.llvm_type(ty);
        let param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = args
            .iter()
            .map(|a| self.llvm_type(a.ty()).into())
            .collect();
        let fn_type = ret_ty.fn_type(&param_types, false);

        let call = self
            .builder
            .build_indirect_call(fn_type, fn_ptr.into_pointer_value(), &arg_vals, "icall")
            .map_err(|e| e.to_string())?;

        // Insert reduction check after indirect call
        self.emit_reduction_check();

        if matches!(ty, MirType::Unit) {
            return Ok(self.context.struct_type(&[], false).const_zero().into());
        }

        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "Indirect call returned void".to_string())
    }

    // ── Closure calls ────────────────────────────────────────────────

    fn codegen_closure_call(
        &mut self,
        closure: &MirExpr,
        args: &[MirExpr],
        ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let closure_val = self.codegen_expr(closure)?;
        let closure_ty = closure_type(self.context);

        // Alloca for the closure struct so we can GEP into it
        let closure_alloca = self
            .builder
            .build_alloca(closure_ty, "closure_tmp")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(closure_alloca, closure_val)
            .map_err(|e| e.to_string())?;

        // Load fn_ptr (field 0)
        let fn_ptr_ptr = self
            .builder
            .build_struct_gep(closure_ty, closure_alloca, 0, "fn_ptr_ptr")
            .map_err(|e| e.to_string())?;
        let fn_ptr = self
            .builder
            .build_load(
                self.context.ptr_type(inkwell::AddressSpace::default()),
                fn_ptr_ptr,
                "fn_ptr",
            )
            .map_err(|e| e.to_string())?;

        // Load env_ptr (field 1)
        let env_ptr_ptr = self
            .builder
            .build_struct_gep(closure_ty, closure_alloca, 1, "env_ptr_ptr")
            .map_err(|e| e.to_string())?;
        let env_ptr = self
            .builder
            .build_load(
                self.context.ptr_type(inkwell::AddressSpace::default()),
                env_ptr_ptr,
                "env_ptr",
            )
            .map_err(|e| e.to_string())?;

        // Build call args: env_ptr first, then user args
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = vec![env_ptr.into()];
        for arg in args {
            let val = self.codegen_expr(arg)?;
            call_args.push(val.into());
        }

        // Build the function type for the indirect call
        let ret_ty = self.llvm_type(ty);
        let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = vec![
            self.context
                .ptr_type(inkwell::AddressSpace::default())
                .into(),
        ];
        for arg in args {
            param_types.push(self.llvm_type(arg.ty()).into());
        }
        let fn_type = ret_ty.fn_type(&param_types, false);

        let call = self
            .builder
            .build_indirect_call(fn_type, fn_ptr.into_pointer_value(), &call_args, "clscall")
            .map_err(|e| e.to_string())?;

        // Insert reduction check after closure call
        self.emit_reduction_check();

        if matches!(ty, MirType::Unit) {
            return Ok(self.context.struct_type(&[], false).const_zero().into());
        }

        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "Closure call returned void".to_string())
    }

    // ── If/else expression ───────────────────────────────────────────

    fn codegen_if(
        &mut self,
        cond: &MirExpr,
        then_body: &MirExpr,
        else_body: &MirExpr,
        ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_val = self.current_function();
        let cond_val = self.codegen_expr(cond)?.into_int_value();

        let result_ty = self.llvm_type(ty);
        let result_alloca = self
            .builder
            .build_alloca(result_ty, "if_result")
            .map_err(|e| e.to_string())?;

        let then_bb = self.context.append_basic_block(fn_val, "then");
        let else_bb = self.context.append_basic_block(fn_val, "else");
        let merge_bb = self.context.append_basic_block(fn_val, "if_merge");

        self.builder
            .build_conditional_branch(cond_val, then_bb, else_bb)
            .map_err(|e| e.to_string())?;

        // Then branch
        self.builder.position_at_end(then_bb);
        let then_val = self.codegen_expr(then_body)?;
        self.builder
            .build_store(result_alloca, then_val)
            .map_err(|e| e.to_string())?;
        // Only branch to merge if current block is not already terminated
        if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        // Else branch
        self.builder.position_at_end(else_bb);
        let else_val = self.codegen_expr(else_body)?;
        self.builder
            .build_store(result_alloca, else_val)
            .map_err(|e| e.to_string())?;
        if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        // Merge block
        self.builder.position_at_end(merge_bb);
        let result = self
            .builder
            .build_load(result_ty, result_alloca, "if_val")
            .map_err(|e| e.to_string())?;

        Ok(result)
    }

    // ── Let binding ──────────────────────────────────────────────────

    fn codegen_let(
        &mut self,
        name: &str,
        ty: &MirType,
        value: &MirExpr,
        body: &MirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let llvm_ty = self.llvm_type(ty);
        let alloca = self
            .builder
            .build_alloca(llvm_ty, name)
            .map_err(|e| e.to_string())?;

        let val = self.codegen_expr(value)?;

        // When binding a runtime-returned pointer to a sum type variable,
        // dereference the pointer to load the actual struct value.
        // Runtime functions like snow_file_read return *mut SnowResult (ptr)
        // but the variable type is SumType (a by-value struct).
        let val = if matches!(ty, MirType::SumType(_))
            && val.is_pointer_value()
            && !llvm_ty.is_pointer_type()
        {
            self.builder
                .build_load(llvm_ty, val.into_pointer_value(), "deref_sum")
                .map_err(|e| e.to_string())?
        } else {
            val
        };

        self.builder
            .build_store(alloca, val)
            .map_err(|e| e.to_string())?;

        // Register the variable
        let old_alloca = self.locals.insert(name.to_string(), alloca);
        let old_type = self.local_types.insert(name.to_string(), ty.clone());

        // Compile the body
        let result = self.codegen_expr(body)?;

        // Restore previous binding (if any)
        if let Some(prev) = old_alloca {
            self.locals.insert(name.to_string(), prev);
        } else {
            self.locals.remove(name);
        }
        if let Some(prev_ty) = old_type {
            self.local_types.insert(name.to_string(), prev_ty);
        } else {
            self.local_types.remove(name);
        }

        Ok(result)
    }

    // ── Block expression ─────────────────────────────────────────────

    fn codegen_block(
        &mut self,
        exprs: &[MirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if exprs.is_empty() {
            return Ok(self.context.struct_type(&[], false).const_zero().into());
        }

        let mut result = self.context.struct_type(&[], false).const_zero().into();
        for expr in exprs {
            result = self.codegen_expr(expr)?;
        }
        Ok(result)
    }

    // ── Pattern match ────────────────────────────────────────────────

    fn codegen_match(
        &mut self,
        scrutinee: &MirExpr,
        arms: &[MirMatchArm],
        ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Evaluate the scrutinee
        let scrutinee_val = self.codegen_expr(scrutinee)?;
        let scrutinee_ty = scrutinee.ty();

        // Alloca for the scrutinee so pattern codegen can GEP into it.
        // For runtime-returned sum types (heap pointers), the value was already
        // dereferenced at the let binding site, so here it's a proper struct value.
        let scrutinee_llvm_ty = self.llvm_type(scrutinee_ty);
        let scrutinee_alloca = if matches!(scrutinee_ty, MirType::SumType(_))
            && scrutinee_val.is_pointer_value()
            && !scrutinee_llvm_ty.is_pointer_type()
        {
            // Rare case: scrutinee is a direct pointer to a sum type
            // (e.g., inline case on a function call result). Use it directly.
            scrutinee_val.into_pointer_value()
        } else {
            let alloca = self
                .builder
                .build_alloca(scrutinee_llvm_ty, "scrutinee")
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(alloca, scrutinee_val)
                .map_err(|e| e.to_string())?;
            alloca
        };

        // Compile pattern to decision tree
        let tree = compile_match(scrutinee_ty, arms, "<unknown>", 0);

        // Alloca for the match result
        let result_ty = self.llvm_type(ty);
        let result_alloca = self
            .builder
            .build_alloca(result_ty, "match_result")
            .map_err(|e| e.to_string())?;

        let fn_val = self.current_function();
        let merge_bb = self.context.append_basic_block(fn_val, "match_merge");

        // Generate code for the decision tree
        self.codegen_decision_tree(
            &tree,
            scrutinee_alloca,
            scrutinee_ty,
            arms,
            result_alloca,
            merge_bb,
        )?;

        // Merge block
        self.builder.position_at_end(merge_bb);
        let result = self
            .builder
            .build_load(result_ty, result_alloca, "match_val")
            .map_err(|e| e.to_string())?;

        Ok(result)
    }

    // ── Struct literal ───────────────────────────────────────────────

    fn codegen_struct_lit(
        &mut self,
        name: &str,
        fields: &[(String, MirExpr)],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let struct_ty = self
            .struct_types
            .get(name)
            .ok_or_else(|| format!("Unknown struct type '{}'", name))?;
        let struct_ty = *struct_ty;

        let alloca = self
            .builder
            .build_alloca(struct_ty, "struct_lit")
            .map_err(|e| e.to_string())?;

        for (i, (_, field_expr)) in fields.iter().enumerate() {
            let val = self.codegen_expr(field_expr)?;
            let field_ptr = self
                .builder
                .build_struct_gep(struct_ty, alloca, i as u32, "field_ptr")
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(field_ptr, val)
                .map_err(|e| e.to_string())?;
        }

        let result = self
            .builder
            .build_load(struct_ty.as_basic_type_enum(), alloca, "struct_val")
            .map_err(|e| e.to_string())?;

        Ok(result)
    }

    // ── Field access ─────────────────────────────────────────────────

    fn codegen_field_access(
        &mut self,
        object: &MirExpr,
        field: &str,
        ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.codegen_expr(object)?;

        // Determine the struct name
        let struct_name = match object.ty() {
            MirType::Struct(name) => name.clone(),
            _ => return Err(format!("Field access on non-struct type: {:?}", object.ty())),
        };

        let struct_ty = self
            .struct_types
            .get(&struct_name)
            .ok_or_else(|| format!("Unknown struct type '{}'", struct_name))?;
        let struct_ty = *struct_ty;

        let field_idx = self.find_struct_field_index(&struct_name, field)?;

        let alloca = self
            .builder
            .build_alloca(struct_ty.as_basic_type_enum(), "obj_tmp")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(alloca, obj_val)
            .map_err(|e| e.to_string())?;

        let field_ptr = self
            .builder
            .build_struct_gep(struct_ty, alloca, field_idx as u32, "field_ptr")
            .map_err(|e| e.to_string())?;

        let result_ty = self.llvm_type(ty);
        let result = self
            .builder
            .build_load(result_ty, field_ptr, "field_val")
            .map_err(|e| e.to_string())?;

        Ok(result)
    }

    /// Find the field index in a struct definition.
    pub(crate) fn find_struct_field_index(&self, struct_name: &str, field: &str) -> Result<usize, String> {
        let fields = self
            .mir_struct_defs
            .get(struct_name)
            .ok_or_else(|| format!("Unknown struct type '{}'", struct_name))?;

        fields
            .iter()
            .position(|(name, _)| name == field)
            .ok_or_else(|| format!("Field '{}' not found in struct '{}'", field, struct_name))
    }

    // ── Sum type variant construction ────────────────────────────────

    fn codegen_construct_variant(
        &mut self,
        type_name: &str,
        variant: &str,
        fields: &[MirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sum_layout = self
            .lookup_sum_type_layout(type_name)
            .ok_or_else(|| format!("Unknown sum type '{}'", type_name))?;
        let sum_layout = *sum_layout;

        let sum_def = self
            .lookup_sum_type_def(type_name)
            .ok_or_else(|| format!("Unknown sum type def '{}'", type_name))?
            .clone();

        let variant_def = sum_def
            .variants
            .iter()
            .find(|v| v.name == variant)
            .ok_or_else(|| format!("Unknown variant '{}.{}'", type_name, variant))?;

        let tag = variant_def.tag;

        // Alloca the sum type
        let alloca = self
            .builder
            .build_alloca(sum_layout.as_basic_type_enum(), "variant")
            .map_err(|e| e.to_string())?;

        // Store the tag at field 0
        let tag_ptr = self
            .builder
            .build_struct_gep(sum_layout, alloca, 0, "tag_ptr")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(tag_ptr, self.context.i8_type().const_int(tag as u64, false))
            .map_err(|e| e.to_string())?;

        // Store fields: create variant overlay struct type and GEP into it
        if !fields.is_empty() {
            let field_types: Vec<MirType> = variant_def.fields.clone();
            let variant_ty =
                variant_struct_type(self.context, &field_types, &self.struct_types, &self.sum_type_layouts);

            // Store each field via the variant overlay
            for (i, field_expr) in fields.iter().enumerate() {
                let val = self.codegen_expr(field_expr)?;
                // GEP into the variant overlay: field 0 is tag, field 1+ are data fields
                let field_ptr = self
                    .builder
                    .build_struct_gep(variant_ty, alloca, (i + 1) as u32, "vfield_ptr")
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_store(field_ptr, val)
                    .map_err(|e| e.to_string())?;
            }
        }

        // Load the complete sum type value
        let result = self
            .builder
            .build_load(sum_layout.as_basic_type_enum(), alloca, "variant_val")
            .map_err(|e| e.to_string())?;

        Ok(result)
    }

    // ── Closure creation ─────────────────────────────────────────────

    fn codegen_make_closure(
        &mut self,
        fn_name: &str,
        captures: &[MirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let cls_ty = closure_type(self.context);
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());

        // Get the function pointer
        let fn_val = self
            .functions
            .get(fn_name)
            .ok_or_else(|| format!("Closure function '{}' not found", fn_name))?;
        let fn_ptr = fn_val.as_global_value().as_pointer_value();

        // Allocate environment on GC heap if there are captures
        let env_ptr = if captures.is_empty() {
            // No captures -> null env pointer
            ptr_ty.const_null()
        } else {
            // Build an env struct type from capture types
            let cap_types: Vec<inkwell::types::BasicTypeEnum<'ctx>> = captures
                .iter()
                .map(|c| self.llvm_type(c.ty()))
                .collect();
            let env_struct_ty = self.context.struct_type(&cap_types, false);

            // Calculate size via target data
            let target_data = inkwell::targets::TargetData::create("");
            let env_size = target_data.get_store_size(&env_struct_ty);

            // Allocate via snow_gc_alloc(size, align=8)
            let gc_alloc = get_intrinsic(&self.module, "snow_gc_alloc");
            let size_val = self.context.i64_type().const_int(env_size, false);
            let align_val = self.context.i64_type().const_int(8, false);
            let env_raw = self
                .builder
                .build_call(gc_alloc, &[size_val.into(), align_val.into()], "env_raw")
                .map_err(|e| e.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("snow_gc_alloc returned void")?;

            let env_ptr_val = env_raw.into_pointer_value();

            // Store each captured value into the env struct
            for (i, cap_expr) in captures.iter().enumerate() {
                let val = self.codegen_expr(cap_expr)?;
                let field_ptr = self
                    .builder
                    .build_struct_gep(env_struct_ty, env_ptr_val, i as u32, "cap_ptr")
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_store(field_ptr, val)
                    .map_err(|e| e.to_string())?;
            }

            env_ptr_val
        };

        // Pack into closure struct { fn_ptr, env_ptr }
        let closure_alloca = self
            .builder
            .build_alloca(cls_ty, "closure")
            .map_err(|e| e.to_string())?;

        let fn_slot = self
            .builder
            .build_struct_gep(cls_ty, closure_alloca, 0, "fn_slot")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(fn_slot, fn_ptr)
            .map_err(|e| e.to_string())?;

        let env_slot = self
            .builder
            .build_struct_gep(cls_ty, closure_alloca, 1, "env_slot")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(env_slot, env_ptr)
            .map_err(|e| e.to_string())?;

        let result = self
            .builder
            .build_load(cls_ty.as_basic_type_enum(), closure_alloca, "closure_val")
            .map_err(|e| e.to_string())?;

        Ok(result)
    }

    // ── Return ───────────────────────────────────────────────────────

    fn codegen_return(
        &mut self,
        inner: &MirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.codegen_expr(inner)?;
        self.builder
            .build_return(Some(&val))
            .map_err(|e| e.to_string())?;
        // Return a dummy value since we've already emitted a return
        Ok(self.context.struct_type(&[], false).const_zero().into())
    }

    // ── Actor primitives ──────────────────────────────────────────────

    fn codegen_actor_spawn(
        &mut self,
        func: &MirExpr,
        args: &[MirExpr],
        priority: u8,
        terminate_callback: Option<&MirExpr>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
        let i64_ty = self.context.i64_type();

        // Get function pointer for the actor entry function.
        let fn_ptr_val = self.codegen_expr(func)?;
        let fn_ptr = if fn_ptr_val.is_pointer_value() {
            fn_ptr_val.into_pointer_value()
        } else {
            // Cast to pointer if needed
            self.builder
                .build_int_to_ptr(fn_ptr_val.into_int_value(), ptr_ty, "fn_ptr")
                .map_err(|e| e.to_string())?
        };

        // Serialize arguments to a byte buffer.
        // Allocate an array of i64 on the stack for args. Each arg is stored
        // as an i64 (ints directly, pointers via ptrtoint).
        let (args_ptr, args_size) = if args.is_empty() {
            (ptr_ty.const_null(), i64_ty.const_int(0, false))
        } else {
            let arg_vals: Vec<BasicValueEnum<'ctx>> = args
                .iter()
                .map(|a| self.codegen_expr(a))
                .collect::<Result<Vec<_>, _>>()?;

            // Allocate [N x i64] on the stack.
            let arr_ty = i64_ty.array_type(arg_vals.len() as u32);
            let buf_alloca = self
                .builder
                .build_alloca(arr_ty, "spawn_args")
                .map_err(|e| e.to_string())?;

            // Store each arg as i64 into the array.
            for (i, val) in arg_vals.iter().enumerate() {
                let int_val = if val.is_int_value() {
                    val.into_int_value()
                } else if val.is_pointer_value() {
                    self.builder
                        .build_ptr_to_int(val.into_pointer_value(), i64_ty, "arg_int")
                        .map_err(|e| e.to_string())?
                } else if val.is_float_value() {
                    self.builder
                        .build_bit_cast(val.into_float_value(), i64_ty, "arg_int")
                        .map_err(|e: inkwell::builder::BuilderError| e.to_string())?
                        .into_int_value()
                } else {
                    // Fallback: store as zero
                    i64_ty.const_int(0, false)
                };
                let idx = self.context.i32_type().const_int(i as u64, false);
                let zero = self.context.i32_type().const_int(0, false);
                let element_ptr = unsafe {
                    self.builder
                        .build_gep(arr_ty, buf_alloca, &[zero, idx], "arg_ptr")
                        .map_err(|e| e.to_string())?
                };
                self.builder
                    .build_store(element_ptr, int_val)
                    .map_err(|e| e.to_string())?;
            }

            let total_size = (arg_vals.len() * 8) as u64;
            (buf_alloca, i64_ty.const_int(total_size, false))
        };

        let priority_val = self.context.i8_type().const_int(priority as u64, false);

        // Call snow_actor_spawn(fn_ptr, args, args_size, priority) -> i64
        let spawn_fn = get_intrinsic(&self.module, "snow_actor_spawn");
        let pid_val = self
            .builder
            .build_call(
                spawn_fn,
                &[fn_ptr.into(), args_ptr.into(), args_size.into(), priority_val.into()],
                "pid",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("snow_actor_spawn returned void")?;

        // If terminate callback exists, call snow_actor_set_terminate(pid, callback_fn_ptr)
        if let Some(cb_expr) = terminate_callback {
            let cb_val = self.codegen_expr(cb_expr)?;
            let cb_ptr = if cb_val.is_pointer_value() {
                cb_val.into_pointer_value()
            } else {
                self.builder
                    .build_int_to_ptr(cb_val.into_int_value(), ptr_ty, "cb_ptr")
                    .map_err(|e| e.to_string())?
            };
            let set_terminate_fn = get_intrinsic(&self.module, "snow_actor_set_terminate");
            self.builder
                .build_call(
                    set_terminate_fn,
                    &[pid_val.into(), cb_ptr.into()],
                    "",
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(pid_val)
    }

    fn codegen_actor_send(
        &mut self,
        target: &MirExpr,
        message: &MirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());

        // Evaluate the target PID (i64).
        let target_val = self.codegen_expr(target)?.into_int_value();

        // Serialize the message to bytes.
        let msg_val = self.codegen_expr(message)?;
        let (msg_ptr, msg_size) = if matches!(message.ty(), MirType::Unit) {
            (ptr_ty.const_null(), i64_ty.const_int(0, false))
        } else {
            // Store the message value on the stack and pass a pointer + size.
            let msg_ty = self.llvm_type(message.ty());
            let msg_alloca = self
                .builder
                .build_alloca(msg_ty, "msg_buf")
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(msg_alloca, msg_val)
                .map_err(|e| e.to_string())?;

            // Compute size via target data.
            let target_data = inkwell::targets::TargetData::create("");
            let size = target_data.get_store_size(&msg_ty);

            (msg_alloca, i64_ty.const_int(size, false))
        };

        // Call snow_actor_send(target_pid, msg_ptr, msg_size)
        let send_fn = get_intrinsic(&self.module, "snow_actor_send");
        self.builder
            .build_call(
                send_fn,
                &[target_val.into(), msg_ptr.into(), msg_size.into()],
                "",
            )
            .map_err(|e| e.to_string())?;

        // Send returns Unit.
        Ok(self.context.struct_type(&[], false).const_zero().into())
    }

    fn codegen_actor_receive(
        &mut self,
        arms: &[MirMatchArm],
        timeout_ms: Option<&MirExpr>,
        result_ty: &MirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());

        // Evaluate timeout: -1 for infinite wait, or the specified value.
        let timeout_val = if let Some(timeout_expr) = timeout_ms {
            self.codegen_expr(timeout_expr)?.into_int_value()
        } else {
            // Infinite wait: -1
            i64_ty.const_int(u64::MAX, true) // -1 as i64
        };

        // Call snow_actor_receive(timeout_ms) -> ptr
        let receive_fn = get_intrinsic(&self.module, "snow_actor_receive");
        let msg_ptr = self
            .builder
            .build_call(receive_fn, &[timeout_val.into()], "msg_ptr")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("snow_actor_receive returned void")?
            .into_pointer_value();

        // Message layout: [u64 type_tag (8 bytes), u64 data_len (8 bytes), u8... data]
        // Skip the 16-byte header to get to the data.
        let data_ptr = unsafe {
            self.builder
                .build_gep(
                    self.context.i8_type(),
                    msg_ptr,
                    &[i64_ty.const_int(16, false)],
                    "data_ptr",
                )
                .map_err(|e| e.to_string())?
        };

        // Load the message data as the expected type.
        // For simple types (Int, Float, Bool), load directly from data_ptr.
        // For String (ptr), load a pointer.
        let msg_val: BasicValueEnum<'ctx> = match result_ty {
            MirType::Int => {
                self.builder
                    .build_load(i64_ty, data_ptr, "msg_int")
                    .map_err(|e| e.to_string())?
            }
            MirType::Float => {
                self.builder
                    .build_load(self.context.f64_type(), data_ptr, "msg_float")
                    .map_err(|e| e.to_string())?
            }
            MirType::Bool => {
                self.builder
                    .build_load(self.context.i8_type(), data_ptr, "msg_bool")
                    .map_err(|e| e.to_string())?
            }
            MirType::String => {
                self.builder
                    .build_load(ptr_ty, data_ptr, "msg_string")
                    .map_err(|e| e.to_string())?
            }
            _ => {
                // For other types, load as i64 (best effort).
                self.builder
                    .build_load(i64_ty, data_ptr, "msg_data")
                    .map_err(|e| e.to_string())?
            }
        };

        // Execute the first matching arm.
        // For now, we support single-arm receive (wildcard/variable binding).
        // More complex multi-arm pattern matching on messages is future work.
        if let Some(arm) = arms.first() {
            // Bind the pattern variable if it's a simple variable pattern.
            match &arm.pattern {
                MirPattern::Var(name, _) => {
                    let alloca = self
                        .builder
                        .build_alloca(msg_val.get_type(), name)
                        .map_err(|e| e.to_string())?;
                    self.builder
                        .build_store(alloca, msg_val)
                        .map_err(|e| e.to_string())?;
                    self.locals.insert(name.clone(), alloca);
                }
                MirPattern::Wildcard => {
                    // No binding needed.
                }
                MirPattern::Literal(_) => {
                    // Literal patterns in receive: just fall through to body.
                    // Full pattern matching support is future work.
                }
                _ => {
                    // For other pattern types (constructor, tuple, etc.), skip binding.
                }
            }

            // Execute the arm body.
            let body_val = self.codegen_expr(&arm.body)?;
            Ok(body_val)
        } else {
            // No arms: return the raw message value.
            Ok(msg_val)
        }
    }

    fn codegen_actor_self(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        // Call snow_actor_self() -> i64
        let self_fn = get_intrinsic(&self.module, "snow_actor_self");
        let result = self
            .builder
            .build_call(self_fn, &[], "self_pid")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("snow_actor_self returned void")?;

        Ok(result)
    }

    fn codegen_actor_link(
        &mut self,
        target: &MirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Evaluate the target PID.
        let target_val = self.codegen_expr(target)?.into_int_value();

        // Call snow_actor_link(target_pid)
        let link_fn = get_intrinsic(&self.module, "snow_actor_link");
        self.builder
            .build_call(link_fn, &[target_val.into()], "")
            .map_err(|e| e.to_string())?;

        // Link returns Unit.
        Ok(self.context.struct_type(&[], false).const_zero().into())
    }

    // ── Reduction check ─────────────────────────────────────────────────

    /// Emit a call to snow_reduction_check() for preemptive scheduling.
    ///
    /// Inserted after function call sites and closure calls to enable
    /// cooperative preemption of actor processes.
    fn emit_reduction_check(&self) {
        if let Some(check_fn) = self.module.get_function("snow_reduction_check") {
            // Only emit if the current block is not yet terminated.
            if let Some(bb) = self.builder.get_insert_block() {
                if bb.get_terminator().is_none() {
                    let _ = self.builder.build_call(check_fn, &[], "");
                }
            }
        }
    }

    // ── Supervisor start ──────────────────────────────────────────────

    fn codegen_supervisor_start(
        &mut self,
        name: &str,
        strategy: u8,
        max_restarts: u32,
        max_seconds: u64,
        children: &[MirChildSpec],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());

        // Build the binary config buffer for snow_supervisor_start.
        // Format: strategy(u8) + max_restarts(u32 LE) + max_seconds(u64 LE) +
        //         child_count(u32 LE) + for each child:
        //           id_len(u32 LE) + id_bytes + fn_ptr_placeholder(u64) +
        //           restart_type(u8) + shutdown_ms(u64 LE) + child_type(u8)
        let mut config_bytes: Vec<u8> = Vec::new();

        // Strategy (1 byte)
        config_bytes.push(strategy);

        // Max restarts (4 bytes LE)
        config_bytes.extend_from_slice(&max_restarts.to_le_bytes());

        // Max seconds (8 bytes LE)
        config_bytes.extend_from_slice(&max_seconds.to_le_bytes());

        // Child count (4 bytes LE)
        config_bytes.extend_from_slice(&(children.len() as u32).to_le_bytes());

        // For each child, we need to embed an offset/placeholder for the function pointer.
        // The fn_ptr will be patched at runtime or we can store a function index.
        // For now, store the child spec metadata; the start function is referenced by name.
        let mut fn_ptr_offsets: Vec<(usize, String)> = Vec::new();

        for child in children {
            // id_len (4 bytes LE)
            let id_bytes = child.id.as_bytes();
            config_bytes.extend_from_slice(&(id_bytes.len() as u32).to_le_bytes());
            // id_bytes
            config_bytes.extend_from_slice(id_bytes);

            // fn_ptr placeholder (8 bytes) -- we'll patch this with a relocation.
            let fn_ptr_offset = config_bytes.len();
            fn_ptr_offsets.push((fn_ptr_offset, child.start_fn.clone()));
            config_bytes.extend_from_slice(&0u64.to_le_bytes()); // placeholder

            // restart_type (1 byte)
            config_bytes.push(child.restart_type);

            // shutdown_ms (8 bytes LE)
            config_bytes.extend_from_slice(&child.shutdown_ms.to_le_bytes());

            // child_type (1 byte)
            config_bytes.push(child.child_type);
        }

        // Create a global constant for the config buffer.
        let config_data = self.context.const_string(&config_bytes, false);
        let config_name = format!(".sup_config_{}", name);
        let config_global = self.module.add_global(config_data.get_type(), None, &config_name);
        config_global.set_initializer(&config_data);
        config_global.set_constant(true);
        config_global.set_unnamed_addr(true);

        // For each child spec, we need to store the function pointer into the config buffer.
        // We do this at runtime by writing the fn_ptr into the config buffer copy on the stack.
        // Actually, since the config is a global constant, we can't patch it.
        // Instead, let's allocate a stack copy and patch fn_ptrs there.
        let config_size = config_bytes.len() as u64;
        let config_size_val = i64_ty.const_int(config_size, false);

        // Allocate stack copy of the config.
        let config_arr_ty = self.context.i8_type().array_type(config_size as u32);
        let config_alloca = self
            .builder
            .build_alloca(config_arr_ty, "sup_config")
            .map_err(|e| e.to_string())?;

        // Memcpy from global to stack.
        let config_global_ptr = config_global.as_pointer_value();
        self.builder
            .build_memcpy(
                config_alloca,
                1,
                config_global_ptr,
                1,
                i64_ty.const_int(config_size, false),
            )
            .map_err(|e| e.to_string())?;

        // Patch function pointers into the stack copy.
        for (offset, fn_name) in &fn_ptr_offsets {
            if fn_name.is_empty() {
                continue;
            }
            // Get the function pointer value.
            let fn_ptr_val = if let Some(fn_val) = self.functions.get(fn_name).copied() {
                fn_val.as_global_value().as_pointer_value()
            } else {
                // Function not found; use null.
                ptr_ty.const_null()
            };

            // Convert fn_ptr to i64.
            let fn_ptr_int = self
                .builder
                .build_ptr_to_int(fn_ptr_val, i64_ty, "fn_ptr_int")
                .map_err(|e| e.to_string())?;

            // GEP to the offset in the config buffer.
            let offset_val = self.context.i32_type().const_int(*offset as u64, false);
            let zero = self.context.i32_type().const_int(0, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_gep(config_arr_ty, config_alloca, &[zero, offset_val], "fn_ptr_slot")
                    .map_err(|e| e.to_string())?
            };

            // Store the fn_ptr as i64 into the config buffer.
            self.builder
                .build_store(elem_ptr, fn_ptr_int)
                .map_err(|e| e.to_string())?;
        }

        // Call snow_supervisor_start(config_ptr, config_size) -> i64 (PID)
        let sup_start_fn = get_intrinsic(&self.module, "snow_supervisor_start");
        let pid_val = self
            .builder
            .build_call(
                sup_start_fn,
                &[config_alloca.into(), config_size_val.into()],
                "sup_pid",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("snow_supervisor_start returned void")?;

        Ok(pid_val)
    }

    // ── Panic ────────────────────────────────────────────────────────

    pub(crate) fn codegen_panic(
        &mut self,
        message: &str,
        file: &str,
        line: u32,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let panic_fn = get_intrinsic(&self.module, "snow_panic");

        // Create global constants for message and file strings
        let msg_val = self.context.const_string(message.as_bytes(), false);
        let msg_global = self.module.add_global(msg_val.get_type(), None, ".panic_msg");
        msg_global.set_initializer(&msg_val);
        msg_global.set_constant(true);

        let file_val = self.context.const_string(file.as_bytes(), false);
        let file_global = self.module.add_global(file_val.get_type(), None, ".panic_file");
        file_global.set_initializer(&file_val);
        file_global.set_constant(true);

        let msg_ptr = msg_global.as_pointer_value();
        let msg_len = self
            .context
            .i64_type()
            .const_int(message.len() as u64, false);
        let file_ptr = file_global.as_pointer_value();
        let file_len = self
            .context
            .i64_type()
            .const_int(file.len() as u64, false);
        let line_val = self.context.i32_type().const_int(line as u64, false);

        self.builder
            .build_call(
                panic_fn,
                &[
                    msg_ptr.into(),
                    msg_len.into(),
                    file_ptr.into(),
                    file_len.into(),
                    line_val.into(),
                ],
                "",
            )
            .map_err(|e| e.to_string())?;

        self.builder
            .build_unreachable()
            .map_err(|e| e.to_string())?;

        // Return a dummy value (unreachable)
        Ok(self.context.i8_type().const_int(0, false).into())
    }
}
