//! Certify the code that can run on a grouped actor's worker.

use inkwell::basic_block::BasicBlock;
use inkwell::values::{CallSiteValue, FunctionValue, InstructionOpcode, Operand};
use mesh_diagnostics::{Code, Diagnostic, SourceFile};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::mir::{diagnostic_at, MirActor, MirNativeFunction, MirSourceSpan, MirSymbol};

pub(super) struct GroupedSpawn<'ctx> {
    pub entry: FunctionValue<'ctx>,
    pub terminate: Option<FunctionValue<'ctx>>,
    /// The `spawn grouped` expression.
    pub at: Option<MirSourceSpan>,
}

#[derive(Clone, Copy)]
enum Execution {
    Actor,
    Terminate,
}

/// What keeps a function from running on a grouped worker.
enum Blocker<'ctx> {
    Native(FunctionValue<'ctx>),
    Unaudited(FunctionValue<'ctx>),
    Recursive(FunctionValue<'ctx>),
    Loop,
    Indirect,
}

/// The calls from a certified root down to the function `blocker` is in.
struct Uncertified<'ctx> {
    calls: Vec<FunctionValue<'ctx>>,
    blocker: Blocker<'ctx>,
}

/// The source's name for an LLVM function where the program defines it,
/// and its symbol otherwise.
struct Names<'a>(&'a FxHashMap<String, MirSymbol>);

impl Names<'_> {
    fn symbol(&self, function: FunctionValue<'_>) -> Option<&MirSymbol> {
        self.0.get(function.get_name().to_string_lossy().as_ref())
    }

    fn of(&self, function: FunctionValue<'_>) -> String {
        self.symbol(function).map_or_else(
            || function.get_name().to_string_lossy().into_owned(),
            |symbol| symbol.source_name.clone(),
        )
    }
}

impl Uncertified<'_> {
    /// One sentence tracing the calls from `root`, which names `calls[0]`:
    /// "`worker` calls `helper`, which makes an unaudited call to `mesh_io_eprintln`".
    fn explain(&self, root: &str, names: &Names<'_>) -> String {
        let mut path = vec![root.to_string()];
        // An actor's entry calls its body, which the source names alike.
        let mut previous = self.calls.first().map(|&root| names.of(root));
        for &function in self.calls.iter().skip(1) {
            let name = names.of(function);
            if previous.as_ref() != Some(&name) {
                path.push(format!("`{name}`"));
                previous = Some(name);
            }
        }
        let symbol =
            |function: FunctionValue<'_>| function.get_name().to_string_lossy().into_owned();
        let problem = match self.blocker {
            Blocker::Native(callee) => format!("makes a native call to `{}`", symbol(callee)),
            Blocker::Unaudited(callee) => {
                format!("makes an unaudited call to `{}`", symbol(callee))
            }
            Blocker::Recursive(callee) => format!(
                "makes a recursive call to `{}` that is not a tail call",
                names.of(callee)
            ),
            Blocker::Loop => "loops, and nothing preempts a terminate callback".to_string(),
            Blocker::Indirect => "makes an indirect or unsupported call".to_string(),
        };
        let (first, rest) = path.split_first().expect("the path starts at its root");
        let calls: String = rest
            .iter()
            .map(|name| format!(" calls {name}, which"))
            .collect();
        format!("{first}{calls} {problem}")
    }
}

pub(super) fn certify(
    spawns: &[GroupedSpawn<'_>],
    actors: &[MirActor],
    natives: &[MirNativeFunction],
    symbols: &FxHashMap<String, MirSymbol>,
    files: &[SourceFile],
) -> Result<(), Diagnostic> {
    if spawns.is_empty() {
        return Ok(());
    }
    let actors: FxHashSet<&str> = actors.iter().map(|actor| actor.entry.as_str()).collect();
    let natives: FxHashSet<&str> = natives
        .iter()
        .map(|native| native.symbol.as_str())
        .collect();
    let names = Names(symbols);
    let mut actor_check = Certificate::new(Execution::Actor, &natives);
    let mut terminate_check = Certificate::new(Execution::Terminate, &natives);
    for spawn in spawns {
        let actor = names.of(spawn.entry);
        if !actors.contains(spawn.entry.get_name().to_string_lossy().as_ref()) {
            return Err(Diagnostic::internal(format!(
                "`spawn grouped` of `{actor}`, which is no actor entry"
            )));
        }
        let fail = |what: &str, root: String, uncertified: Uncertified<'_>| {
            let note = uncertified.explain(&root, &names);
            let diagnostic = diagnostic_at(
                files,
                spawn.at,
                Code::SpawnGroupedUnsafe,
                format!("`spawn grouped` cannot certify {what}{root}"),
                "spawned grouped here",
            )
            .with_note(note);
            // The deepest function the program defines on the path.
            let defined = uncertified.calls.iter().rev().find_map(|&function| {
                let at = names.symbol(function)?.definition?;
                Some((files.get(at.file.0)?, at.span, names.of(function)))
            });
            match defined {
                Some((file, span, name)) => {
                    diagnostic.with_label_in(file, span, format!("`{name}` is defined here"))
                }
                None => diagnostic,
            }
        };
        actor_check
            .visit(spawn.entry)
            .map_err(|uncertified| fail("actor ", format!("`{actor}`"), uncertified))?;
        if let Some(callback) = spawn.terminate {
            terminate_check.visit(callback).map_err(|uncertified| {
                fail(
                    "",
                    format!("the terminate callback of `{actor}`"),
                    uncertified,
                )
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

    fn visit(&mut self, function: FunctionValue<'ctx>) -> Result<(), Uncertified<'ctx>> {
        if self.certified.contains(&function) {
            return Ok(());
        }
        let blocked =
            |calls: Vec<FunctionValue<'ctx>>, blocker| Err(Uncertified { calls, blocker });

        let name = function.get_name().to_string_lossy();
        if self.natives.contains(name.as_ref()) {
            return blocked(Vec::new(), Blocker::Native(function));
        }
        let Some(entry) = function.get_first_basic_block() else {
            return if allowed_leaf(self.execution, &name) {
                Ok(())
            } else {
                blocked(Vec::new(), Blocker::Unaudited(function))
            };
        };

        if !self.active.insert(function) {
            return blocked(Vec::new(), Blocker::Recursive(function));
        }
        if matches!(self.execution, Execution::Terminate)
            && has_cycle(entry, &mut FxHashSet::default(), &mut FxHashSet::default())
        {
            return blocked(vec![function], Blocker::Loop);
        }

        for instruction in function
            .get_basic_block_iter()
            .flat_map(BasicBlock::get_instructions)
        {
            match instruction.get_opcode() {
                InstructionOpcode::Call | InstructionOpcode::Invoke | InstructionOpcode::CallBr => {
                    let Some(callee) = CallSiteValue::try_from(instruction)
                        .ok()
                        .and_then(CallSiteValue::get_called_fn_value)
                    else {
                        return blocked(vec![function], Blocker::Indirect);
                    };
                    self.visit(callee).map_err(|mut uncertified| {
                        uncertified.calls.insert(0, function);
                        uncertified
                    })?;
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
        | "mesh_reduction_check"
        | "mesh_reduction_state_v1" => true,
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
        | "mesh_panic_at"
        | "mesh_panic_str"
        | "mesh_timer_sleep" => matches!(execution, Execution::Actor),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use inkwell::context::Context;
    use mesh_diagnostics::{Code, Diagnostic, SourceFile};
    use std::collections::{HashMap, HashSet};

    use crate::codegen::CodeGen;
    use crate::mir::lower::lower_module_with_diagnostics;
    use crate::mir::MirSourceId;

    fn compile(source: &str) -> Result<String, Diagnostic> {
        compile_steps(source, false)
    }

    fn compile_steps(source: &str, enabled: bool) -> Result<String, Diagnostic> {
        compile_placements(source, "", enabled, false)
    }

    /// The LLVM IR of `source` as the module `module_name`, its spans
    /// pointing into `main.mpl`.
    fn compile_placements(
        source: &str,
        module_name: &str,
        grouped: bool,
        movable: bool,
    ) -> Result<String, Diagnostic> {
        let parse = mesh_parser::parse(source);
        assert!(parse.ok(), "{:?}", parse.errors());
        let checked = mesh_typeck::check(&parse);
        assert!(checked.errors.is_empty(), "{:?}", checked.errors);
        let files = [SourceFile::new("main.mpl", source)];
        let lowered = lower_module_with_diagnostics(
            &parse,
            &checked,
            module_name,
            &HashSet::new(),
            &HashMap::new(),
            &[],
            |_| Some(MirSourceId(0)),
            &files,
            false,
        )?;
        if let Some(error) = lowered.diagnostics.into_iter().next() {
            return Err(error);
        }
        let context = Context::create();
        let mut codegen = CodeGen::new(&context, "grouped_spawn", 0, None).unwrap();
        codegen.set_source_files(&files);
        codegen.grouped_step_enabled = grouped;
        codegen.movable_step_enabled = movable;
        codegen.compile(&lowered.module)?;
        Ok(codegen.get_llvm_ir())
    }

    /// The `E0106` rejection of `source`: its message, its one note, and the
    /// text its span covers.
    fn rejection(source: &str) -> (String, String, String) {
        rejection_in(source, "")
    }

    fn rejection_in(source: &str, module_name: &str) -> (String, String, String) {
        let error = compile_placements(source, module_name, false, false).unwrap_err();
        assert_eq!(error.code(), Code::SpawnGroupedUnsafe, "{error:?}");
        let span = error.span().expect("the rejection points at its spawn");
        let note = match error.notes() {
            [note] => note.clone(),
            notes => panic!("{notes:?}"),
        };
        (
            error.message().to_string(),
            note,
            source[span.start as usize..span.end as usize].to_string(),
        )
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
        let (message, note, spawn) = rejection(
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
        );
        assert_eq!(message, "`spawn grouped` cannot certify actor `worker`");
        assert_eq!(
            note,
            "`worker` calls `count`, which makes a recursive call to `count` that is not a tail call"
        );
        assert_eq!(spawn, "spawn grouped(worker, 1000000)");
    }

    #[test]
    fn grouped_spawn_rejects_transitive_native_blocking_and_indirect_calls() {
        for (helper, call, expected) in [
            (
                "@native(\"custom_wait\")\npub fn wait() -> Int\nfn helper() -> Int = wait()",
                "helper()",
                "`worker` calls `helper`, which makes a native call to `custom_wait`",
            ),
            (
                "fn helper() do\n IO.eprintln(\"blocking\")\nend",
                "helper()",
                "`worker` calls `helper`, which makes an unaudited call to `mesh_io_eprintln`",
            ),
            (
                "fn helper() do\n Process.exit(1)\nend",
                "helper()",
                "`worker` calls `helper`, which makes an unaudited call to `mesh_process_exit`",
            ),
            (
                "@native(\"mesh_monotonic_now_nanos\")\npub fn clock() -> Int\nfn helper() -> Int = clock()",
                "helper()",
                "`worker` calls `helper`, which makes a native call to `mesh_monotonic_now_nanos`",
            ),
            (
                "fn helper(f :: Fun() -> Int) -> Int = f()",
                "helper(fn -> 1 end)",
                "`worker` calls `helper`, which makes an indirect or unsupported call",
            ),
        ] {
            let source = format!("{helper}\nactor worker() do\n {call}\nend\nfn main() do spawn grouped(worker) end");
            let (message, note, spawn) = rejection(&source);
            assert_eq!(message, "`spawn grouped` cannot certify actor `worker`");
            assert_eq!(note, expected);
            assert_eq!(spawn, "spawn grouped(worker)");
            compile(&source.replace("spawn grouped", "spawn")).unwrap();
        }
    }

    #[test]
    fn grouped_spawn_checks_forward_declared_terminate_callback() {
        for (body, expected) in [
            (
                "helper()",
                "the terminate callback of `worker` calls `helper`, which makes a native call to `custom_wait`",
            ),
            (
                "Timer.sleep(1)",
                "the terminate callback of `worker` makes an unaudited call to `mesh_timer_sleep`",
            ),
            (
                "while true do\n  0\nend",
                "the terminate callback of `worker` loops, and nothing preempts a terminate callback",
            ),
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
            let (message, note, _) = rejection(&source);
            assert_eq!(
                message,
                "`spawn grouped` cannot certify the terminate callback of `worker`"
            );
            assert_eq!(note, expected);
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
        let (message, _, spawn) = rejection(source);
        assert_eq!(
            message,
            "`spawn grouped` cannot start child `w` of supervisor `Sup`"
        );
        assert_eq!(spawn, "spawn grouped(worker)");
        compile(&source.replace("spawn grouped", "spawn")).unwrap();
    }

    /// A module's private function has a mangled symbol; the note names it
    /// as the source does, and a label points at its definition.
    #[test]
    fn grouped_spawn_names_functions_as_the_source_does() {
        let source = "fn helper() do\n  IO.eprintln(\"blocking\")\nend\nactor worker() do\n  helper()\nend\nfn main() do\n  spawn grouped(worker)\nend\n";
        let error = compile_placements(source, "Workers", false, false).unwrap_err();
        let (_, note, _) = rejection_in(source, "Workers");
        assert_eq!(
            note,
            "`worker` calls `Workers.helper`, which makes an unaudited call to `mesh_io_eprintln`"
        );
        assert!(!note.contains("Workers__helper"), "{note}");
        let [label] = error.labels() else {
            panic!("{:?}", error.labels());
        };
        let span = label.span();
        assert_eq!(&source[span.start as usize..span.end as usize], "helper");
        assert_eq!(label.message(), "`Workers.helper` is defined here");
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
    fn ordinary_receive_step_requires_local_pid_provenance() {
        let source = suite_counter().replace("spawn grouped", "spawn");
        let ir = compile_placements(&source, "", false, true).unwrap();
        assert!(ir.contains("call i64 @mesh_actor_spawn_movable_step(ptr @counter_loop, ptr @counter_loop.step, i32 2,"));
        assert!(ir.lines().any(
            |line| line.contains("call i64 @mesh_actor_spawn_movable_step(")
                && line.ends_with("i64 1, i8 1)")
        ));
        let fallback = source.replace(
            "send(caller, updated)",
            "send(if updated == 0 do self() else caller end, updated)",
        );
        let ir = compile_placements(&fallback, "", false, true).unwrap();
        assert!(ir.contains("call i64 @mesh_actor_spawn_shaped(ptr @counter_loop,"));
        assert!(!ir.contains("call i64 @mesh_actor_spawn_movable_step("));
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
        for hook in [
            "mesh_actor_spawn_step",
            "mesh_actor_spawn_movable_step",
            "mesh_step_charge",
        ] {
            let source =
                format!("@native(\"{hook}\")\npub fn foreign() -> Int\nfn main() do foreign() end");
            for enabled in [false, true] {
                let error = compile_steps(&source, enabled).unwrap_err();
                assert_eq!(error.code(), Code::RuntimeSymbolCollision);
                let error = error.message();
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
        assert_eq!(err.code(), Code::SpawnGroupedUnsafe);
        let err = &err.notes()[0];
        assert!(err.contains("indirect or unsupported call"), "{err}");
    }
}
