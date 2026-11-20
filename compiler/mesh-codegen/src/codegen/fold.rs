use inkwell::builder::BuilderError;
use inkwell::values::{FunctionValue, IntValue};
use inkwell::IntPredicate;

use crate::mir::{BinOp, MirExpr, MirFunction, MirType};

use super::{intrinsics::get_intrinsic, CodeGen};

/// A tail loop over an immutable list with an integer accumulator. Only
/// wrapping arithmetic and reads at the current index may run in a chunk.
struct IntListFold<'a> {
    index: &'a str,
    accumulator: &'a str,
    list: &'a str,
    update: &'a MirExpr,
}

fn single(expr: &MirExpr) -> &MirExpr {
    match expr {
        MirExpr::Block(exprs, _) if exprs.len() == 1 => single(&exprs[0]),
        expr => expr,
    }
}

fn is_var(expr: &MirExpr, expected: &str) -> bool {
    matches!(single(expr), MirExpr::Var(name, _) if name == expected)
}

impl<'a> IntListFold<'a> {
    fn recognize(function: &'a MirFunction) -> Option<Self> {
        let [(list, MirType::Ptr), (end, MirType::Int), (index, MirType::Int), (accumulator, MirType::Int)] =
            function.params.as_slice()
        else {
            return None;
        };
        if function.return_type != MirType::Int {
            return None;
        }
        let MirExpr::If {
            cond,
            then_body,
            else_body,
            ..
        } = single(&function.body)
        else {
            return None;
        };
        let MirExpr::BinOp {
            op: BinOp::Eq,
            lhs,
            rhs,
            ..
        } = single(cond)
        else {
            return None;
        };
        if !(is_var(lhs, index) && is_var(rhs, end) || is_var(lhs, end) && is_var(rhs, index))
            || !is_var(then_body, accumulator)
        {
            return None;
        }
        let MirExpr::TailCall { args, .. } = single(else_body) else {
            return None;
        };
        let [next_list, next_end, next_index, update] = args.as_slice() else {
            return None;
        };
        if !is_var(next_list, list)
            || !is_var(next_end, end)
            || !matches!(
                single(next_index),
                MirExpr::BinOp { op: BinOp::Add, lhs, rhs, ty: MirType::Int, .. }
                    if is_var(lhs, index) && matches!(single(rhs), MirExpr::IntLit(1, _))
            )
        {
            return None;
        }
        let fold = Self {
            index,
            accumulator,
            list,
            update,
        };
        let mut reads_element = false;
        (fold.check_update(update, &mut reads_element) && reads_element).then_some(fold)
    }

    fn check_update(&self, expr: &MirExpr, reads_element: &mut bool) -> bool {
        match single(expr) {
            MirExpr::IntLit(..) => true,
            MirExpr::Var(name, MirType::Int) => name == self.index || name == self.accumulator,
            MirExpr::BinOp {
                op: BinOp::Add | BinOp::Sub | BinOp::Mul,
                lhs,
                rhs,
                ty: MirType::Int,
                ..
            } => self.check_update(lhs, reads_element) && self.check_update(rhs, reads_element),
            MirExpr::Call {
                func,
                args,
                ty: MirType::Int,
                ..
            } if is_var(func, "mesh_list_get") => {
                let [list, index] = args.as_slice() else {
                    return false;
                };
                *reads_element = true;
                is_var(list, self.list) && is_var(index, self.index)
            }
            _ => false,
        }
    }

    fn emit_update<'ctx>(
        &self,
        cg: &CodeGen<'ctx>,
        expr: &MirExpr,
        index: IntValue<'ctx>,
        accumulator: IntValue<'ctx>,
        element: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, BuilderError> {
        match single(expr) {
            MirExpr::IntLit(value, _) => Ok(cg.context.i64_type().const_int(*value as u64, true)),
            MirExpr::Var(name, _) if name == self.index => Ok(index),
            MirExpr::Var(..) => Ok(accumulator),
            MirExpr::Call { .. } => Ok(element),
            MirExpr::BinOp { op, lhs, rhs, .. } => {
                let lhs = self.emit_update(cg, lhs, index, accumulator, element)?;
                let rhs = self.emit_update(cg, rhs, index, accumulator, element)?;
                match op {
                    BinOp::Add => cg.builder.build_int_add(lhs, rhs, "fold_add"),
                    BinOp::Sub => cg.builder.build_int_sub(lhs, rhs, "fold_sub"),
                    BinOp::Mul => cg.builder.build_int_mul(lhs, rhs, "fold_mul"),
                    _ => unreachable!("uncertified fold arithmetic"),
                }
            }
            _ => unreachable!("uncertified fold expression"),
        }
    }
}

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn codegen_int_list_fold(
        &self,
        mir: &MirFunction,
        function: FunctionValue<'ctx>,
    ) -> Result<(), BuilderError> {
        let Some(fold) = IntListFold::recognize(mir) else {
            return Ok(());
        };
        if self.functions.contains_key("mesh_list_get") {
            return Ok(());
        }
        let [list, end, start, initial]: [_; 4] = function.get_params().try_into().unwrap();
        let list = list.into_pointer_value();
        let end = end.into_int_value();
        let start = start.into_int_value();
        let initial = initial.into_int_value();
        let b = &self.builder;
        let word = self.context.i64_type();
        let counter = self.context.i32_type();
        let block = |name| self.context.append_basic_block(function, name);
        let range = block("fold_range");
        let setup = block("fold_setup");
        let chunk = block("fold_chunk");
        let body = block("fold_body");
        let charge = block("fold_charge");
        let debit = block("fold_debit");
        let slow = block("fold_safepoint");
        let next = block("fold_next");
        let done = block("fold_done");
        let fallback = block("fold_fallback");
        let advancing = b.build_int_compare(IntPredicate::ULT, start, end, "fold_advancing")?;
        b.build_conditional_branch(advancing, range, fallback)?;

        // Preserve the original loop and its bounds diagnostics for every
        // input outside a provably valid, nonempty interval.
        b.position_at_end(range);
        let length = b.build_load(word, list, "fold_length")?.into_int_value();
        let fits = b.build_int_compare(IntPredicate::ULE, end, length, "fold_fits")?;
        b.build_conditional_branch(fits, setup, fallback)?;

        b.position_at_end(setup);
        let state = self
            .codegen_runtime_call("mesh_reduction_state_v1", &[], "fold_reductions")
            .into_pointer_value();
        b.build_unconditional_branch(chunk)?;

        b.position_at_end(chunk);
        let chunk_start = b.build_phi(word, "fold_start")?;
        chunk_start.add_incoming(&[(&start, setup)]);
        let chunk_total = b.build_phi(word, "fold_total")?;
        chunk_total.add_incoming(&[(&initial, setup)]);
        let first = chunk_start.as_basic_value().into_int_value();
        let budget = b
            .build_load(counter, state, "fold_budget")?
            .into_int_value();
        let zero = b.build_int_compare(
            IntPredicate::EQ,
            budget,
            counter.const_zero(),
            "fold_empty_budget",
        )?;
        let limit = b.build_select(
            zero,
            counter.const_int(1, false),
            budget,
            "fold_budget_limit",
        )?;
        let limit = b.build_int_z_extend(limit.into_int_value(), word, "fold_budget_word")?;
        let limit = self.fold_min(limit, word.const_int(1024, false))?;
        let remaining = b.build_int_sub(end, first, "fold_remaining")?;
        let count = self.fold_min(remaining, limit)?;
        let last = b.build_int_add(first, count, "fold_chunk_end")?;
        // Keep the list live across any safepoint. Resolve a view's backing
        // storage inside each chunk, where the arithmetic cannot suspend.
        let data = self.build_list_data(b, list)?;
        let data_block = b.get_insert_block().unwrap();
        b.build_unconditional_branch(body)?;

        b.position_at_end(body);
        let index = b.build_phi(word, "fold_index")?;
        let total = b.build_phi(word, "fold_accumulator")?;
        index.add_incoming(&[(&first, data_block)]);
        total.add_incoming(&[(&chunk_total.as_basic_value(), data_block)]);
        let current = index.as_basic_value().into_int_value();
        let slot = unsafe { b.build_gep(word, data, &[current], "fold_slot")? };
        let element = b.build_load(word, slot, "fold_element")?.into_int_value();
        let updated = fold.emit_update(
            self,
            fold.update,
            current,
            total.as_basic_value().into_int_value(),
            element,
        )?;
        let incremented = b.build_int_add(current, word.const_int(1, false), "fold_increment")?;
        index.add_incoming(&[(&incremented, body)]);
        total.add_incoming(&[(&updated, body)]);
        let chunk_done =
            b.build_int_compare(IntPredicate::EQ, incremented, last, "fold_chunk_done")?;
        b.build_conditional_branch(chunk_done, charge, body)?;

        // A zero budget still executes one iteration before its check, just
        // like the original tail loop. An idle host leaves its GC word alone.
        b.position_at_end(charge);
        b.build_switch(
            budget,
            debit,
            &[
                (counter.const_zero(), slow),
                (counter.const_all_ones(), next),
            ],
        )?;
        b.position_at_end(debit);
        let count = b.build_int_truncate(count, counter, "fold_charge_count")?;
        let rest = b.build_int_sub(budget, count, "fold_budget_rest")?;
        b.build_store(state, rest)?;
        b.build_unconditional_branch(next)?;

        b.position_at_end(slow);
        b.build_call(get_intrinsic(&self.module, "mesh_reduction_check"), &[], "")?;
        b.build_unconditional_branch(next)?;
        b.position_at_end(next);
        chunk_start.add_incoming(&[(&last, next)]);
        chunk_total.add_incoming(&[(&updated, next)]);
        let complete = b.build_int_compare(IntPredicate::EQ, last, end, "fold_complete")?;
        b.build_conditional_branch(complete, done, chunk)?;
        b.position_at_end(done);
        b.build_return(Some(&updated))?;

        b.position_at_end(fallback);
        Ok(())
    }

    fn fold_min(
        &self,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, BuilderError> {
        let less = self
            .builder
            .build_int_compare(IntPredicate::ULT, lhs, rhs, "fold_less")?;
        Ok(self
            .builder
            .build_select(less, lhs, rhs, "fold_min")?
            .into_int_value())
    }
}
