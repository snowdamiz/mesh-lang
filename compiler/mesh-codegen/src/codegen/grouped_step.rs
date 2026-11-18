//! Lower a single untimed receive to one synchronous mailbox step. The only
//! live values across steps are the word parameters of the actor body.

use inkwell::module::Linkage;
use inkwell::values::{
    BasicValue, CallSiteValue, FunctionValue, InstructionOpcode, IntValue, PointerValue,
};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{CodeGen, BUILT};
use crate::mir::{MirExpr, MirModule, MirPattern, MirType, MsgShape};

pub(super) struct SpawnSite<'ctx> {
    pub actor: String,
    pub original: PointerValue<'ctx>,
    pub args: PointerValue<'ctx>,
    pub size: IntValue<'ctx>,
    pub shape: PointerValue<'ctx>,
    pub pid: IntValue<'ctx>,
}

pub(super) fn frame_word(ty: &MirType) -> bool {
    matches!(ty, MirType::Int | MirType::Pid(_))
}

fn local_value(ty: &MirType) -> bool {
    frame_word(ty)
        || matches!(
            ty,
            MirType::Bool | MirType::Unit | MirType::String | MirType::Never
        )
}

// Named helpers run on the worker stack. Only accept branch-only leaf helpers
// with scalar operations and audited panic calls. A helper calling another
// Mesh function also contains a coroutine reduction check and must fall back.
fn safe_helper(f: FunctionValue<'_>, natives: &FxHashSet<&str>) -> bool {
    let Some(entry) = f.get_first_basic_block() else {
        return false;
    };
    if super::group_safety::has_cycle(entry, &mut FxHashSet::default(), &mut FxHashSet::default()) {
        return false;
    }
    f.get_basic_block_iter()
        .flat_map(|b| b.get_instructions())
        .all(|ins| {
            use InstructionOpcode::*;
            match ins.get_opcode() {
                Alloca | Store | Load | Br | Return | Unreachable | Add | Sub | Mul | ICmp
                | Xor | And | Or | Trunc | ZExt | SExt => true,
                Call => CallSiteValue::try_from(ins)
                    .ok()
                    .and_then(CallSiteValue::get_called_fn_value)
                    .is_some_and(|callee| {
                        let name = callee.get_name().to_string_lossy();
                        callee.count_basic_blocks() == 0
                            && !natives.contains(name.as_ref())
                            && matches!(name.as_ref(), "mesh_panic_str" | "mesh_panic")
                    }),
                _ => false,
            }
        })
}

struct Handler<'a> {
    params: &'a [(String, MirType)],
    binding: Option<&'a str>,
    body: &'a MirExpr,
}

fn handler<'a>(mir: &'a MirModule, entry: &str) -> Option<Handler<'a>> {
    mir.actors
        .iter()
        .find(|a| a.entry == entry && a.terminate.is_none())?;
    let wrapper = mir.functions.iter().find(|f| f.name == entry)?;
    if !matches!(&wrapper.params[..], [(name, MirType::Ptr)] if name == "__args_ptr")
        || !matches!(wrapper.body, MirExpr::Unit)
    {
        return None;
    }
    let body_name = format!("__actor_{entry}_body");
    let f = mir.functions.iter().find(|f| f.name == body_name)?;
    if !f.has_tail_calls || f.params.is_empty() || !f.params.iter().all(|(_, ty)| frame_word(ty)) {
        return None;
    }
    let mut receive = &f.body;
    while let MirExpr::Block(seq, _) = receive {
        let [only] = seq.as_slice() else { return None };
        receive = only;
    }
    let MirExpr::ActorReceive {
        handler: Some((temp, MirType::Int, dispatch)),
        timeout_ms: None,
        timeout_body: None,
        ..
    } = receive
    else {
        return None;
    };
    let MirExpr::Match {
        scrutinee, arms, ..
    } = dispatch.as_ref()
    else {
        return None;
    };
    let [arm] = arms.as_slice() else { return None };
    if scrutinee.var_name() != Some(temp.as_str()) || arm.guard.is_some() {
        return None;
    }
    // The ordinary dispatch binds its internal receive name before the arm.
    if f.params.iter().any(|(name, _)| name == temp) {
        return None;
    }
    let binding = match &arm.pattern {
        MirPattern::Var(name, MirType::Int) => Some(name.as_str()),
        MirPattern::Wildcard => None,
        _ => return None,
    };
    Some(Handler {
        params: &f.params,
        binding,
        body: &arm.body,
    })
}

impl<'ctx> CodeGen<'ctx> {
    fn step_expression_supported(
        &self,
        expr: &MirExpr,
        handler: &Handler<'_>,
        mir: &MirModule,
        natives: &FxHashSet<&str>,
    ) -> bool {
        use MirExpr::*;
        if !local_value(expr.ty()) {
            return false;
        }
        let allowed = match expr {
            Unit
            | IntLit(..)
            | BoolLit(..)
            | StringLit(..)
            | Var(..)
            | Panic { .. }
            | ActorSelf { .. }
            | Block(..)
            | If { .. }
            | Let { .. }
            | UnaryOp { .. } => true,
            BinOp { op, lhs, rhs, .. } => {
                !matches!(
                    op,
                    crate::mir::BinOp::Concat | crate::mir::BinOp::Div | crate::mir::BinOp::Mod
                ) && [lhs.ty(), rhs.ty()]
                    .iter()
                    .all(|t| frame_word(t) || matches!(t, MirType::Bool))
            }
            Shaped {
                shape: MsgShape::Scalar | MsgShape::Pid,
                ..
            } => true,
            ActorSend { message, .. } => matches!(message.ty(), MirType::Int),
            TailCall { args, .. } => {
                args.len() == handler.params.len()
                    && args
                        .iter()
                        .zip(handler.params)
                        .all(|(a, (_, ty))| a.ty() == ty)
            }
            Call { func, args, .. } => {
                let Some(name) = func.var_name() else {
                    return false;
                };
                if natives.contains(name) {
                    return false;
                }
                let Some(helper) = mir.functions.iter().find(|f| f.name == name) else {
                    return false;
                };
                return !helper.is_closure_fn
                    && !helper.has_tail_calls
                    && helper.captures.is_empty()
                    && helper.params.iter().all(|(_, t)| local_value(t))
                    && local_value(&helper.return_type)
                    && safe_helper(self.functions[name], natives)
                    && args
                        .iter()
                        .all(|arg| self.step_expression_supported(arg, handler, mir, natives));
            }
            _ => false,
        };
        allowed
            && expr
                .children()
                .iter()
                .all(|child| self.step_expression_supported(child, handler, mir, natives))
    }

    pub(super) fn lower_grouped_steps(&mut self, mir: &MirModule) -> Result<(), String> {
        if self.step_sites.is_empty() {
            return Ok(());
        }
        let natives: FxHashSet<&str> = mir
            .native_functions
            .iter()
            .flat_map(|f| [f.name.as_str(), f.symbol.as_str()])
            .collect();
        let mut compiled = FxHashMap::default();
        for site in std::mem::take(&mut self.step_sites) {
            let Some(h) = handler(mir, &site.actor) else {
                continue;
            };
            if site.size.get_zero_extended_constant() != Some(h.params.len() as u64 * 8) {
                continue;
            }
            let (entry, bound) = if let Some(pair) = compiled.get(&site.actor) {
                *pair
            } else {
                if !self.step_expression_supported(h.body, &h, mir, &natives) {
                    continue;
                }
                let entry = self.emit_step(&site.actor, h)?;
                let Some(bound) = segment_bound(entry, &natives) else {
                    // The handler is private and no spawn references it yet.
                    self.builder.clear_insertion_position();
                    self.current_fn = None;
                    self.locals.clear();
                    unsafe { entry.delete() };
                    continue;
                };
                let pair = (entry, bound);
                compiled.insert(site.actor.clone(), pair);
                pair
            };
            let old = site.pid.as_instruction_value().expect("grouped spawn call");
            self.builder.position_before(&old);
            let result = self.codegen_runtime_call(
                "mesh_actor_spawn_step",
                &[
                    site.original.into(),
                    entry.as_global_value().as_pointer_value().into(),
                    self.context
                        .i32_type()
                        .const_int(bound.into(), false)
                        .into(),
                    site.args.into(),
                    site.size.into(),
                    site.shape.into(),
                ],
                "step_pid",
            );
            site.pid.replace_all_uses_with(result.into_int_value());
            old.erase_from_basic_block();
        }
        Ok(())
    }

    fn emit_step(&mut self, actor: &str, h: Handler<'_>) -> Result<FunctionValue<'ctx>, String> {
        let ptr = self.context.ptr_type(Default::default());
        let status = self.context.i8_type();
        let word = self.context.i64_type();
        let f = self.module.add_function(
            &format!("{actor}.step"),
            status.fn_type(&[ptr.into(), ptr.into()], false),
            Some(Linkage::Internal),
        );
        self.current_fn = Some(f);
        self.locals.clear();
        self.local_types.clear();
        self.tce_param_slots.clear();
        self.builder
            .position_at_end(self.context.append_basic_block(f, "entry"));
        let frame = f.get_nth_param(0).unwrap().into_pointer_value();
        let payload = f.get_nth_param(1).unwrap().into_pointer_value();
        for (i, (name, ty)) in h.params.iter().enumerate() {
            let slot = unsafe {
                self.builder.build_gep(
                    word,
                    frame,
                    &[word.const_int(i as u64, false)],
                    "frame_slot",
                )
            }
            .expect(BUILT);
            self.locals.insert(name.clone(), slot);
            self.local_types.insert(name.clone(), ty.clone());
            self.tce_param_slots.push(slot);
        }
        if let Some(name) = h.binding {
            let value = self
                .builder
                .build_load(word, payload, "message")
                .expect(BUILT);
            self.bind_local(name, &MirType::Int, value)?;
        }
        self.emitting_step = true;
        let result = self.codegen_expr(h.body);
        self.emitting_step = false;
        self.tce_param_slots.clear();
        result?;
        if self.block_is_open() {
            self.builder
                .build_return(Some(&status.const_zero()))
                .expect(BUILT);
        }
        Ok(f)
    }
}

// Check emitted calls as well as MIR. Builtin expansion and future codegen
// changes must not introduce a suspension or an unbounded cycle on the worker.
fn segment_bound(f: FunctionValue<'_>, natives: &FxHashSet<&str>) -> Option<u32> {
    if super::group_safety::has_cycle(
        f.get_first_basic_block()?,
        &mut FxHashSet::default(),
        &mut FxHashSet::default(),
    ) {
        return None;
    }
    let mut charges = 0u32;
    for ins in f.get_basic_block_iter().flat_map(|b| b.get_instructions()) {
        use InstructionOpcode::*;
        match ins.get_opcode() {
            Call => {
                let callee = CallSiteValue::try_from(ins).ok()?.get_called_fn_value()?;
                let name = callee.get_name().to_string_lossy();
                if natives.contains(name.as_ref()) {
                    return None;
                }
                match name.as_ref() {
                    "mesh_step_charge" => charges = charges.checked_add(1)?,
                    "mesh_actor_self" | "mesh_actor_send" | "mesh_panic" | "mesh_panic_str" => (),
                    _ if safe_helper(callee, natives) => (),
                    _ => return None,
                }
            }
            Alloca | Store | Load | Br | Return | Unreachable | Add | Sub | Mul | ICmp | Xor
            | And | Or | Trunc | ZExt | SExt | GetElementPtr | Phi | Select => (),
            _ => return None,
        }
    }
    (charges > 0).then_some(charges)
}
