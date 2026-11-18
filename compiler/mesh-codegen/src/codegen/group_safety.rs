//! Certify the code that can run on a grouped actor's worker.

use inkwell::basic_block::BasicBlock;
use inkwell::values::{CallSiteValue, FunctionValue, InstructionOpcode, Operand};
use rustc_hash::FxHashSet;

use crate::mir::MirNativeFunction;

pub(super) struct GroupedSpawn<'ctx> {
    pub entry: FunctionValue<'ctx>,
    pub terminate: Option<FunctionValue<'ctx>>,
}

#[derive(Clone, Copy)]
enum Execution {
    Actor,
    Terminate,
}

pub(super) fn certify(
    spawns: &[GroupedSpawn<'_>],
    actors: &[crate::mir::MirActor],
    natives: &[MirNativeFunction],
) -> Result<(), String> {
    if spawns.is_empty() {
        return Ok(());
    }
    let actors: FxHashSet<&str> = actors.iter().map(|actor| actor.entry.as_str()).collect();
    let natives: FxHashSet<&str> = natives
        .iter()
        .map(|native| native.symbol.as_str())
        .collect();
    let mut actor_check = Certificate::new(Execution::Actor, &natives);
    let mut terminate_check = Certificate::new(Execution::Terminate, &natives);
    for spawn in spawns {
        let actor = spawn.entry.get_name().to_string_lossy();
        if !actors.contains(actor.as_ref()) {
            return Err(format!(
                "`spawn grouped` requires an actor entry, found `{actor}`"
            ));
        }
        actor_check
            .visit(spawn.entry)
            .map_err(|error| format!("`spawn grouped` cannot certify actor `{actor}`: {error}"))?;
        if let Some(callback) = spawn.terminate {
            terminate_check.visit(callback).map_err(|error| {
                format!("`spawn grouped` cannot certify terminate callback of `{actor}`: {error}")
            })?;
        }
    }
    Ok(())
}

struct Certificate<'ctx, 'a> {
    execution: Execution,
    natives: &'a FxHashSet<&'a str>,
    certified: FxHashSet<FunctionValue<'ctx>>,
    active: FxHashSet<FunctionValue<'ctx>>,
}

impl<'ctx, 'a> Certificate<'ctx, 'a> {
    fn new(execution: Execution, natives: &'a FxHashSet<&'a str>) -> Self {
        Self {
            execution,
            natives,
            certified: FxHashSet::default(),
            active: FxHashSet::default(),
        }
    }

    fn visit(&mut self, function: FunctionValue<'ctx>) -> Result<(), String> {
        if self.certified.contains(&function) {
            return Ok(());
        }

        let name = function.get_name().to_string_lossy();
        if self.natives.contains(name.as_ref()) {
            return Err(format!("native call to `{name}`"));
        }
        let Some(entry) = function.get_first_basic_block() else {
            return if allowed_leaf(self.execution, &name) {
                Ok(())
            } else {
                Err(format!("unaudited call to `{name}`"))
            };
        };

        if !self.active.insert(function) {
            return Err(format!(
                "recursive call to `{name}` without a loop back-edge"
            ));
        }
        if matches!(self.execution, Execution::Terminate)
            && has_cycle(entry, &mut FxHashSet::default(), &mut FxHashSet::default())
        {
            return Err(format!("loop in `{name}` outside an actor coroutine"));
        }

        for instruction in function
            .get_basic_block_iter()
            .flat_map(BasicBlock::get_instructions)
        {
            match instruction.get_opcode() {
                InstructionOpcode::Call | InstructionOpcode::Invoke | InstructionOpcode::CallBr => {
                    let callee = CallSiteValue::try_from(instruction)
                        .ok()
                        .and_then(CallSiteValue::get_called_fn_value)
                        .ok_or_else(|| format!("indirect or unsupported call in `{name}`"))?;
                    self.visit(callee)
                        .map_err(|error| format!("`{name}` calls {error}"))?;
                }
                _ => {}
            }
        }
        self.active.remove(&function);
        self.certified.insert(function);
        Ok(())
    }
}

// The runtime invokes terminate callbacks without a yielder, so even
// compiler-inserted reduction checks cannot preempt a loop there.
pub(super) fn has_cycle<'ctx>(
    block: BasicBlock<'ctx>,
    active: &mut FxHashSet<BasicBlock<'ctx>>,
    done: &mut FxHashSet<BasicBlock<'ctx>>,
) -> bool {
    if done.contains(&block) {
        return false;
    }
    if !active.insert(block) {
        return true;
    }
    let cyclic = block
        .get_terminator()
        .into_iter()
        .flat_map(|terminator| terminator.get_operands())
        .filter_map(|operand| operand.and_then(Operand::block))
        .any(|successor| has_cycle(successor, active, done));
    active.remove(&block);
    done.insert(block);
    cyclic
}

fn allowed_leaf(execution: Execution, name: &str) -> bool {
    // Permit only the declarations audited for this execution context.
    // In particular, receive and sleep take synchronous paths without a
    // coroutine, and send needs the grouped actor context for its remote guard.
    match name {
        "llvm.memcpy.p0.p0.i64"
        | "mesh_gc_alloc_actor"
        | "mesh_list_append"
        | "mesh_monotonic_now_nanos"
        | "mesh_reduction_check" => true,
        "mesh_actor_receive"
        | "mesh_actor_receive_scalar"
        | "mesh_actor_wait_timeout"
        | "mesh_actor_self"
        | "mesh_actor_send"
        | "mesh_actor_send_shaped"
        | "mesh_actor_spawn"
        | "mesh_actor_spawn_shaped"
        | "mesh_actor_spawn_grouped"
        | "mesh_actor_set_terminate"
        | "mesh_actor_stop"
        | "mesh_panic"
        | "mesh_panic_str"
        | "mesh_timer_sleep" => matches!(execution, Execution::Actor),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use inkwell::context::Context;
    use std::collections::{HashMap, HashSet};

    use crate::codegen::CodeGen;
    use crate::mir::lower::lower_to_mir;

    fn compile(source: &str) -> Result<String, String> {
        compile_steps(source, false)
    }

    fn compile_steps(source: &str, enabled: bool) -> Result<String, String> {
        let parse = mesh_parser::parse(source);
        assert!(parse.ok(), "{:?}", parse.errors());
        let checked = mesh_typeck::check(&parse);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let mir = lower_to_mir(&parse, &checked, "", &HashSet::new(), &HashMap::new())?;
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "grouped_spawn", 0, None).unwrap();
        codegen.grouped_step_enabled = enabled;
        codegen.compile(&mir)?;
        Ok(codegen.get_llvm_ir())
    }

    #[test]
    fn grouped_spawn_certifies_recursive_and_shared_descendants_and_emits_abi() {
        let ir = compile(
            r#"
fn count(n :: Int, total :: Int) -> Int do
  if n <= 0 do
    total
  else
    count(n - 1, total + 1)
  end
end
fn left(n :: Int) -> Int = count(n, 0)
fn right(n :: Int) -> Int = count(n, 0)
actor empty() do
  Timer.sleep(1)
  terminate do
    0
  end
end
actor worker(text :: String, n :: Int) do
  let _total = left(n) + right(n)
  spawn grouped(empty)
  spawn(empty)
end
fn main() do
  spawn grouped(worker, "argument", 2)
end
"#,
        )
        .unwrap();
        assert!(ir.contains("call i64 @count("));
        assert!(ir.contains(
            "call i64 @mesh_actor_spawn_grouped(ptr @empty, ptr null, i64 0, ptr null, ptr @__terminate.empty)"
        ));
        assert!(ir.contains("call i64 @mesh_actor_spawn(ptr @empty, ptr null, i64 0, i8 1)"));
        assert!(ir.lines().any(|line| {
            line.contains("call i64 @mesh_actor_spawn_grouped(ptr @worker")
                && line.contains("i64 16, ptr @")
        }));
        assert!(ir.lines().any(|line| {
            line.contains("call void @mesh_actor_set_terminate(")
                && line.contains("ptr @__terminate.empty")
        }));
    }

    #[test]
    fn grouped_spawn_rejects_recursive_stack_growth() {
        let error = compile(
            r#"
fn count(n :: Int) -> Int do
  if n == 0 do
    0
  else
    1 + count(n - 1)
  end
end
actor worker(n :: Int) do
  count(n)
end
fn main() do
  spawn grouped(worker, 1000000)
end
"#,
        )
        .unwrap_err();
        assert!(error.contains("recursive call to `count`"), "{error}");
    }

    #[test]
    fn grouped_spawn_rejects_transitive_native_blocking_and_indirect_calls() {
        for (helper, call, expected) in [
            (
                "@native(\"custom_wait\")\npub fn wait() -> Int\nfn helper() -> Int = wait()",
                "helper()",
                "native call to `custom_wait`",
            ),
            (
                "fn helper() do\n IO.eprintln(\"blocking\")\nend",
                "helper()",
                "unaudited call to `mesh_io_eprintln`",
            ),
            (
                "fn helper() do\n Process.exit(1)\nend",
                "helper()",
                "unaudited call to `mesh_process_exit`",
            ),
            (
                "@native(\"mesh_monotonic_now_nanos\")\npub fn clock() -> Int\nfn helper() -> Int = clock()",
                "helper()",
                "native call to `mesh_monotonic_now_nanos`",
            ),
            (
                "fn helper(f :: Fun() -> Int) -> Int = f()",
                "helper(fn -> 1 end)",
                "indirect or unsupported call in `helper`",
            ),
        ] {
            let source = format!("{helper}\nactor worker() do\n {call}\nend\nfn main() do spawn grouped(worker) end");
            let error = compile(&source).unwrap_err();
            assert!(error.contains("cannot certify actor `worker`"), "{error}");
            assert!(error.contains(expected), "{error}");
            compile(&source.replace("spawn grouped", "spawn")).unwrap();
        }
    }

    #[test]
    fn grouped_spawn_checks_forward_declared_terminate_callback() {
        for (body, expected) in [
            ("helper()", "native call to `custom_wait`"),
            ("Timer.sleep(1)", "unaudited call to `mesh_timer_sleep`"),
            ("while true do\n  0\nend", "loop in `__terminate.worker`"),
        ] {
            let source = format!(
                r#"
@native("custom_wait")
pub fn wait() -> Int
fn helper() -> Int = wait()
fn main() do
  spawn grouped(worker)
end
actor worker() do
  0
  terminate do
    {body}
  end
end
"#
            );
            let error = compile(&source).unwrap_err();
            assert!(
                error.contains("cannot certify terminate callback of `worker`"),
                "{error}"
            );
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn grouped_spawn_is_rejected_for_supervisor_child_start() {
        let source = r#"
actor worker() do
  0
end
supervisor Sup do
  strategy: one_for_one
  child w do
    start: fn -> spawn grouped(worker) end
    restart: permanent
    shutdown: 5000
  end
end
"#;
        let error = compile(source).unwrap_err();
        assert!(
            error.contains("cannot start child `w` of supervisor `Sup`"),
            "{error}"
        );
        compile(&source.replace("spawn grouped", "spawn")).unwrap();
    }
    fn suite_counter() -> String {
        let suite = include_str!("../../../../tools/performance/mesh/main.mpl");
        let start = suite.find("fn check_actor_reply(").unwrap();
        let end = suite[start..].find("actor pair_caller(").unwrap();
        format!("{}\nactor driver() do\n spawn grouped(counter_loop, self(), 0, 2)\nend\nfn main() do\n spawn(driver)\nend", &suite[start..start+end])
    }

    #[test]
    fn grouped_step_compiles_suite_counter_and_named_helper() {
        for actor in ["counter_loop", "running_total"] {
            let source = suite_counter().replace("counter_loop", actor);
            let legacy = compile_steps(&source, false).unwrap();
            assert!(legacy.contains("call i64 @mesh_actor_spawn_grouped("));
            assert!(!legacy.contains("call i64 @mesh_actor_spawn_step("));

            let ir = compile_steps(&source, true).unwrap();
            assert!(ir.contains(&format!(
                "call i64 @mesh_actor_spawn_step(ptr @{actor}, ptr @{actor}.step, i32 2,"
            )));
            let (_, step) = ir
                .split_once(&format!("define internal i8 @{actor}.step("))
                .unwrap();
            let (body, _) = step.split_once("\n}").unwrap();
            assert!(body.contains("call {} @check_actor_reply"));
            assert_eq!(body.matches("call void @mesh_step_charge()").count(), 2);
            assert!(body.contains("ret i8 0") && body.contains("ret i8 1"));
            assert!(!body.contains("mesh_actor_receive") && !body.contains("mesh_reduction_check"));
        }
    }

    #[test]
    fn grouped_step_preserves_fallback_for_ineligible_handlers() {
        let source = suite_counter();
        for candidate in [
            source.replace("total + amount", "total / amount"),
            source.replace("total + amount", "total % amount"),
            source.replace("panic(message)", "Timer.sleep(1)"),
            format!(
                "fn pause() do Timer.sleep(1) end\n{}",
                source.replace("panic(message)", "pause()")
            ),
            source.replace(
                "      end\n  end\nend",
                "      end\n    after 10 -> Timer.sleep(1)\n  end\nend",
            ),
            source.replace("    amount ->", "    0 -> Timer.sleep(1)\n    amount ->"),
            source.replace(
                "      end\n  end\nend",
                "      end\n  end\n  terminate do\n    0\n  end\nend",
            ),
            source.replace(
                "let updated = total + amount",
                "Timer.sleep(1)\n      let updated = total + amount",
            ),
        ] {
            assert_ne!(candidate, source);
            let ir = compile_steps(&candidate, true).unwrap();
            assert!(ir.contains("call i64 @mesh_actor_spawn_grouped(ptr @counter_loop"));
            assert!(!ir.contains("call i64 @mesh_actor_spawn_step("));
        }
    }
    #[test]
    fn grouped_step_reserves_its_runtime_symbols_in_both_modes() {
        for hook in ["mesh_actor_spawn_step", "mesh_step_charge"] {
            let source =
                format!("@native(\"{hook}\")\npub fn foreign() -> Int\nfn main() do foreign() end");
            for enabled in [false, true] {
                let error = compile_steps(&source, enabled).unwrap_err();
                assert!(
                    error.contains("reserved for grouped actor codegen"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn grouped_step_keeps_receive_temporary_shadowing() {
        let template = suite_counter().replace("total", "__recv_msg_000");
        let offset = template.find("receive do").unwrap();
        let name = format!("__recv_msg_{offset}");
        assert_eq!(name.len(), "__recv_msg_000".len());
        let ir = compile_steps(&template.replace("__recv_msg_000", &name), true).unwrap();
        assert!(ir.contains("call i64 @mesh_actor_spawn_grouped(ptr @counter_loop"));
        assert!(!ir.contains("call i64 @mesh_actor_spawn_step("));
    }

    #[test]
    fn grouped_step_keeps_group_certificate_for_callback_calls() {
        let source = format!(
            "fn apply(f :: Fun() -> Int) -> Int = f()\n{}",
            suite_counter().replace(
                "let updated = total + amount",
                "let updated = total + amount + apply(fn -> 0 end)"
            )
        );
        let err = compile_steps(&source, true).unwrap_err();
        assert!(err.contains("indirect or unsupported call"), "{err}");
    }
}
