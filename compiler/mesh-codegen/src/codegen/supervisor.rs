use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::module::Linkage;
use inkwell::passes::PassBuilderOptions;
use inkwell::values::GlobalValue;
use inkwell::values::{CallSiteValue, FunctionValue, InstructionOpcode};
use rustc_hash::FxHashSet;

use mesh_diagnostics::Diagnostic;

use super::{CodeGen, BUILT};

fn calls(function: FunctionValue<'_>) -> impl Iterator<Item = CallSiteValue<'_>> {
    function.get_basic_block_iter().flat_map(|block| {
        block
            .get_instructions()
            .filter_map(|ins| CallSiteValue::try_from(ins).ok())
    })
}

pub(super) struct ConfigTemplate<'ctx> {
    pub global: GlobalValue<'ctx>,
    pub bytes: Vec<u8>,
    pub worker_tags: Vec<(usize, FunctionValue<'ctx>)>,
}

// A placement hint for short message handlers. Loops must pass through a
// receive, including loops inside helpers. Unknown calls take the shared queue.
fn message_worker<'ctx>(entry: FunctionValue<'ctx>, natives: &FxHashSet<&str>) -> bool {
    let mut analysis = MessageWork {
        natives,
        seen: FxHashSet::default(),
        active: FxHashSet::default(),
        receives: false,
        instructions_left: 512,
    };
    analysis.visit(entry) && analysis.receives
}

struct MessageWork<'ctx, 'a> {
    natives: &'a FxHashSet<&'a str>,
    seen: FxHashSet<FunctionValue<'ctx>>,
    active: FxHashSet<FunctionValue<'ctx>>,
    receives: bool,
    instructions_left: usize,
}

impl<'ctx> MessageWork<'ctx, '_> {
    fn visit(&mut self, function: FunctionValue<'ctx>) -> bool {
        if self.seen.contains(&function) {
            return true;
        }
        if !self.active.insert(function) {
            return false;
        }
        let mut boundaries = FxHashSet::default();
        for block in function.get_basic_block_iter() {
            for ins in block.get_instructions() {
                if self.instructions_left == 0 {
                    return false;
                }
                self.instructions_left -= 1;
                match ins.get_opcode() {
                    InstructionOpcode::Invoke | InstructionOpcode::CallBr => return false,
                    InstructionOpcode::Call => {}
                    _ => continue,
                }
                let Some(callee) = CallSiteValue::try_from(ins)
                    .ok()
                    .and_then(CallSiteValue::get_called_fn_value)
                else {
                    return false;
                };
                let name = callee.get_name().to_string_lossy();
                if self.natives.contains(name.as_ref()) {
                    return false;
                }
                if callee.count_basic_blocks() > 0 {
                    if !self.visit(callee) {
                        return false;
                    }
                    continue;
                }
                match name.as_ref() {
                    "mesh_actor_receive" | "mesh_actor_receive_scalar" => {
                        self.receives = true;
                        boundaries.insert(block);
                    }
                    "mesh_actor_self"
                    | "mesh_actor_stop"
                    | "mesh_actor_send"
                    | "mesh_actor_send_shaped"
                    | "mesh_process_whereis"
                    | "mesh_reduction_check"
                    | "mesh_reduction_state_v1"
                    | "mesh_panic"
                    | "mesh_panic_at"
                    | "mesh_panic_str"
                    | "mesh_panic_str_at"
                    | "mesh_process_exit"
                    | "llvm.memcpy.p0.p0.i64" => {}
                    // A diagnostic immediately followed by process exit cannot
                    // turn this child into a long-lived I/O worker.
                    "mesh_io_eprintln"
                        if ins
                            .get_next_instruction()
                            .and_then(|next| CallSiteValue::try_from(next).ok())
                            .and_then(CallSiteValue::get_called_fn_value)
                            .is_some_and(|next| {
                                next.get_name().to_bytes() == b"mesh_process_exit"
                            }) => {}
                    _ => return false,
                }
            }
        }
        let mut active: FxHashSet<BasicBlock<'ctx>> = FxHashSet::default();
        for block in function.get_basic_block_iter() {
            if super::group_safety::has_cycle(block, &mut active, &mut boundaries) {
                return false;
            }
        }
        self.active.remove(&function);
        self.seen.insert(function);
        true
    }
}

impl CodeGen<'_> {
    pub(super) fn mark_message_workers(&mut self, natives: &[crate::mir::MirNativeFunction]) {
        let natives = natives.iter().map(|f| f.symbol.as_str()).collect();
        for mut config in self.supervisor_configs.drain(..) {
            for (tag, entry) in config.worker_tags {
                if message_worker(entry, &natives) {
                    config.bytes[tag] |= 0x80;
                }
            }
            config
                .global
                .set_initializer(&self.context.const_string(&config.bytes, false));
        }
    }

    /// Let a supervised child whose entry ends in `panic(...)` exit through
    /// `mesh_actor_fail[_at]` instead of unwinding back to the scheduler.
    pub(super) fn lower_supervisor_failures(&self) -> Result<(), Diagnostic> {
        // Each panic intrinsic and the failure that replaces it take the same
        // arguments.
        let lowered: Vec<_> = [
            ("mesh_panic_str", "mesh_actor_fail"),
            ("mesh_panic_str_at", "mesh_actor_fail_at"),
        ]
        .into_iter()
        .filter_map(|(panic, fail)| {
            Some((
                self.module.get_function(panic)?,
                self.module.get_function(fail)?,
            ))
        })
        .collect();
        let is_panic =
            |f: Option<FunctionValue<'_>>| f.is_some_and(|f| lowered.iter().any(|&(p, _)| p == f));
        let inline = self
            .context
            .create_enum_attribute(Attribute::get_named_enum_kind_id("alwaysinline"), 0);
        let mut entries = Vec::new();
        let builder = self.context.create_builder();
        for &(entry, store) in &self.supervisor_entries {
            let mut has_panic = false;
            for call in calls(entry) {
                let Some(body) = call.get_called_fn_value() else {
                    continue;
                };
                if is_panic(Some(body)) {
                    has_panic = true;
                } else if calls(body).any(|c| is_panic(c.get_called_fn_value())) {
                    call.add_attribute(AttributeLoc::Function, inline);
                    has_panic = true;
                }
            }
            if !has_panic {
                continue;
            }

            // Only this private entry can return directly to the scheduler.
            // The same actor body or start function called elsewhere must unwind.
            let root = self.module.add_function(
                "supervised.start",
                entry.get_type(),
                Some(Linkage::Private),
            );
            builder.position_at_end(self.context.append_basic_block(root, "entry"));
            let call = builder
                .build_call(entry, &[root.get_first_param().unwrap().into()], "")
                .expect(BUILT);
            call.add_attribute(AttributeLoc::Function, inline);
            builder
                .build_return(Some(&self.context.struct_type(&[], false).const_zero()))
                .expect(BUILT);
            let address = root
                .as_global_value()
                .as_pointer_value()
                .const_to_int(self.context.i64_type());
            assert!(store.set_operand(0, address));
            entries.push(root);
        }
        if entries.is_empty() {
            return Ok(());
        }
        self.module
            .run_passes(
                "always-inline,function(simplifycfg)",
                &self.target_machine,
                PassBuilderOptions::create(),
            )
            .map_err(Diagnostic::internal)?;
        for entry in entries {
            for block in entry.get_basic_block_iter() {
                let Some(end) = block.get_terminator() else {
                    continue;
                };
                if end.get_opcode() != InstructionOpcode::Unreachable {
                    continue;
                }
                let Some(ins) = end.get_previous_instruction() else {
                    continue;
                };
                let called = CallSiteValue::try_from(ins)
                    .ok()
                    .and_then(CallSiteValue::get_called_fn_value);
                let Some(&(_, fail)) = lowered.iter().find(|&&(p, _)| Some(p) == called) else {
                    continue;
                };
                // The callee is the last operand; the arguments precede it.
                let args: Vec<_> = (0..ins.get_num_operands() - 1)
                    .map(|i| ins.get_operand(i).unwrap().value().unwrap().into())
                    .collect();
                builder.position_before(&ins);
                builder.build_call(fail, &args, "").expect(BUILT);
                builder
                    .build_return(Some(&self.context.struct_type(&[], false).const_zero()))
                    .expect(BUILT);
                end.erase_from_basic_block();
                ins.erase_from_basic_block();
            }
        }
        self.verify()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwell::context::Context;
    use inkwell::memory_buffer::MemoryBuffer;

    #[test]
    fn only_receive_delimited_cycles_get_the_placement_hint() {
        let context = Context::create();
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                br#"
            declare ptr @mesh_actor_receive(i64)
            define void @messages() {
              entry: br label %next
              next: call ptr @mesh_actor_receive(i64 -1)
              br label %next
            }
            define void @cpu(i1 %done) {
              entry: br label %work
              work: br i1 %done, label %end, label %work
              end: ret void
            }
            define void @calls_cpu() {
              entry: call ptr @mesh_actor_receive(i64 -1)
              call void @cpu(i1 true)
              ret void
            }
            define void @spins_after_receiving(i1 %done) {
              entry: br label %next
              next: call ptr @mesh_actor_receive(i64 -1)
              br label %work
              work: br i1 %done, label %next, label %work
            }
        "#,
                "message_workers",
            ))
            .unwrap();
        let eligible = |name, natives| message_worker(module.get_function(name).unwrap(), natives);
        let natives = FxHashSet::default();
        assert!(eligible("messages", &natives));
        assert!(!eligible("cpu", &natives));
        assert!(!eligible("calls_cpu", &natives));
        assert!(!eligible("spins_after_receiving", &natives));
        let foreign = FxHashSet::from_iter(["mesh_actor_receive"]);
        assert!(!eligible("messages", &foreign));
    }
}
