//! MIR expression to LLVM IR translation.
//!
//! Implements `codegen_expr` which translates each MIR expression variant
//! into corresponding LLVM IR instructions.
//!
//! Full implementation in Task 2.

use inkwell::values::BasicValueEnum;

use super::CodeGen;
use crate::mir::MirExpr;

impl<'ctx> CodeGen<'ctx> {
    /// Generate LLVM IR for a MIR expression.
    ///
    /// Returns the LLVM value representing the expression result.
    pub(crate) fn codegen_expr(
        &mut self,
        expr: &MirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match expr {
            MirExpr::Unit => {
                Ok(self.context.struct_type(&[], false).const_zero().into())
            }
            _ => {
                // Stub: remaining expression types implemented in Task 2
                Ok(self.context.struct_type(&[], false).const_zero().into())
            }
        }
    }
}
