# Native stack containment feasibility

This is an early proof unit for the server runtime work. It does not implement
actor isolation. The inspected source baseline is
`7a7feda2594593eb6318c17cfb612c8016a99342`.

LLVM 21.1.8 can report excessive fixed machine frames through a context diagnostic
handler before the application publishes or executes the generated code. That
handler can make Mesh compilation fail. LLVM's emission and MCJIT APIs do not
fail automatically, even when the handler emits an error diagnostic.

`warn-stack-size` alone does not certify maximum native stack consumption.
The probes reproduce omissions for dynamic allocations, stack realignment,
red-zone use, and optimized x86 outgoing arguments. Probes touch guard pages
before a body-level entry check runs. They do not turn exhaustion into an unwind.
The proposed unrestricted combination is therefore insufficient.

The smallest candidate worth continuing is a restricted Darwin arm64 emitter
with a machine-code stack bound, an admission check on every entry, and an audited
runtime-call budget. The exact contract below is conditional. This unit proves
individual mechanisms and supplies counterexamples, not its full sufficiency.
Pre-prologue stubs are the next candidate if the machine-code bound cannot be
certified. Explicit heap continuations are a broader fallback.

## Reproduce the observations

Run from this checkout on Darwin arm64 with the installed LLVM 21 and Rust tools.

```sh
LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21 python3 proof/stack-containment/run.py
```

The driver runs serially. Each child has a 30-second CPU limit, a 45-second wall
timeout, and disabled core dumps. It never runs cross-target code. The native
allocation examples use bounded sizes. The unwind example uses two sequential
256 KiB Rust threads. It raises an ordinary unwind above the physical limit.
No signal handler, longjmp, guard-page recovery, network service, or provider
mutation is involved. The named-probe rejection cases intentionally abort only
their `llc` child processes.

Inputs are in `proof/stack-containment/`. Full commands, LLVM IR, assembly, final
machine IR, logs, and binaries are regenerated under `.tmp/stack-containment/run/`.
`results.json` is written only after the entire experiment succeeds. The retained
`proof/stack-containment/observed.json` records one run, including input hashes,
LLVM and Rust versions, the source commit, and output. It is historical evidence.
Rerun the command for a new toolchain or source baseline.

The 24 main cases cover four targets, O0 and O2, and three attribute settings.
O2 runs `opt -passes=default<O2>` before `llc -O2`. Settings are no probe attribute,
`probe-stack=inline-asm`, and that probe attribute plus `noredzone`. Every case
reports positive fixed frame sizes with `warn-stack-size=0`. Separate cap tests
use 4096 bytes and the exact Darwin leaf-frame boundary. This is diagnostic
instrumentation, not a proposed production cap.

The additional MCJIT cases use its O0 and O2 codegen settings without a separate
IR optimization pipeline. That matches the separate concern at the REPL boundary.
The source comment in `frames.ll` is replaced with an 800-argument call by the
driver. The generated `inline.ll` is the complete standalone reproduction input.

## What was observed

| Target | O0 and O2 codegen | Large true leaf with inline probe attribute | Runtime execution |
| --- | --- | --- | --- |
| Darwin arm64 | Completed | 4096-byte steps with stack touches | Bounded SP measurement, object diagnostics, MCJIT diagnostics, and synthetic Rust unwind |
| Linux arm64 | Completed | 4096-byte steps with stack touches | Not run |
| Linux x86_64 | Completed | Inline page-touch sequence | Not run |
| Windows x86_64 MSVC | Completed | Calls the literal symbol `inline-asm` | Not run |

On Windows, omitting the custom attribute emits the ABI's `__chkstk` call before
the stack adjustment. Naming `__mesh_probe` emits a call to that symbol on both
x86 targets. Neither named implementation was supplied or executed. On both arm64
targets, naming `__mesh_probe` instead of `inline-asm` aborts LLVM with
`Unsupported stack probing method`. The attribute is not a portable hook for a
custom function that raises a Mesh unwind. These results agree with the pinned
[AArch64 attribute implementation](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/AArch64/AArch64MachineFunctionInfo.cpp#L99-L128).

Without the probe attribute, the Darwin and Linux arm64 large leaf allocates its
frame in one step. Both direct and indirect call functions, an external runtime
call, and the true leaf receive inline probes when requested. The true leaf
has no machine call. Its inline assembly operand only keeps the allocation live.

### Reported frames and actual use differ

The retained Darwin O2 run reports and measures 32,784 bytes for the true leaf.
The direct, indirect, and external-runtime-call functions each report and measure
32,800 bytes. The spill overhead is visible in the final assembly. These examples
show that the warning is later than IR alloca sizing.
The measurement returns `llvm.stacksave.p0` from the body before the epilogue.
These sizes describe the exact probe functions, not every possible Mesh function.

The counterexamples establish why that success cannot be generalized.

* A 16-byte alloca aligned to 65,536 bytes reports a 65,536-byte frame on Darwin.
  Its measured SP change exceeds that value because of the final alignment mask.
  The exact measurement varies with the initial SP. The assembly establishes the
  missing alignment adjustment independently of address randomization.
* A variable alloca of 32,768 bytes reports only 16 bytes at Darwin O2.
  A constant 1024-byte alloca inside a loop also reports 16 bytes while 32 loop
  iterations consume 32,784 bytes. At O0 the respective reported fixed sizes are
  32 and 64 bytes. Neither form triggers a 4096-byte cap warning. A module with
  only the dynamic example is accepted by both object emission and MCJIT.
* Linux x86_64 uses the red zone for a 64-byte leaf without adjusting SP. The
  warning reports no positive fixed frame. `noredzone` changes the reported size
  to 72 bytes. A larger leaf similarly reports a smaller frame with the red zone.
* Linux x86_64 O2 reports 8 bytes for `outgoing`, even with `noredzone` and probes.
  The machine IR records `maxCallFrameSize: 6352`. Assembly contains one frame push
  and 794 argument pushes before the call. Thus a 4096-byte frame warning passes
  a function that transiently consumes more than 6 KiB, even without dynamic IR
  allocas or over-aligned locals. O0 reports 6360 bytes. The experiment with
  `-no-x86-call-frame-opt` restores 6360 at O2, but that global backend option is
  not itself a complete safety contract.

LLVM computes this warning from the fixed `MachineFrameInfo::getStackSize()`
after prologue insertion and register scavenging. It does not define that value
as every stack address a function can touch. See the pinned
[warning site](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/PrologEpilogInserter.cpp#L272-L290)
and [x86 call-frame optimization](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86CallFrameOptimization.cpp#L125-L139).

The generated `body_check` reads SP before other IR body operations. Its final
assembly still allocates and probes the entire frame before comparing SP with the
limit. An IR instruction at function entry does not execute before the prologue.

### Diagnostics need an explicit publication gate

`diagnostics.cpp` installs `LLVMContextSetDiagnosticHandler` and identifies
`DiagnosticInfoStackSize` using the LLVM C++ type. Both object and JIT cases
observe the excessive-frame diagnostics at O0 and O2.

For object emission, `LLVMTargetMachineEmitToMemoryBuffer` returns zero and
produces a nonempty buffer. The driver records rejection and discards that buffer.
For MCJIT, engine creation succeeds with zero frame diagnostics. Function lookup
generates the code, delivers the diagnostics, and returns a nonzero address.
The driver records rejection and destroys the engine without calling that address.

The `object-error` and `jit-error` cases emit an LLVM error from inside the
warning handler. `HasErrors` becomes true, but emission still returns success and
MCJIT still returns an address. The API return value alone is insufficient. This
matches the [LLVM context handler behavior](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/IR/LLVMContext.cpp#L225-L246).

The Darwin leaf reports 32,784 bytes. A threshold of 32,783 rejects it and a
threshold of 32,784 accepts it. The threshold is inclusive. Handler state must
outlive all lazy compilation and stay associated with the exact emitted module.
No Rust panic or C++ exception may escape the diagnostic callback into LLVM.

### Ordinary unwinding works in the native synthetic example

`unwind.ll` allocates a 4096-byte local, probes, checks a limit, and calls a Rust
bridge either directly or indirectly. The bridge owns a destructor marker and
recurses through the generated entry. Below the synthetic threshold, a
`C-unwind` Rust helper calls `resume_unwind`. The Rust thread catches the marker.

At both codegen levels, both call forms enter and drop 50 bridge markers. The
reported SP at failure is 61,760 bytes above the thread's usable lower bound.
The process continues after both threads. The generated O2 frame is 4144 bytes.
The example uses a deliberately generous 64 KiB logical reserve to exercise
ordinary unwinding. That number does not bound the system unwinder, Rust runtime,
panic payload allocation, cleanup code, or Mesh finalization. It is not a proposed
production reserve. This example neither runs an actor nor verifies corosensei.

## Boundaries in the current Mesh source

`compiler/mesh-rt/src/stack_overflow.rs` installs alternate signal stacks and
reports guard faults by aborting. `GUARD_WINDOW` is only fault classification.
It is not a containment reserve.

`actor/stack.rs` wraps `Coroutine::resume` with `on_stack` and catches ordinary
unwinds in `resume_catching_panic`. The surrounding `on_stack` call restores the
previous limit when resume returns after a yield or normal completion. The catch
path explicitly restores the thread limit after an unwind. Keep both behaviors
when introducing an execution limit. The actor's limit must not remain installed
while the worker runs another actor or scheduler code.

The pinned corosensei dependency is 0.3.2. Its Unix `DefaultStack::limit()` returns
the mapping base, including the inaccessible guard page. Its first writable
address is one page above that value. A safe budget must use the writable bound,
not blindly add a reserve to the current `stack_limit` field. Windows stack bounds
need their own committed-versus-reserved interpretation. The real default stack
constant in `actor/process.rs:262` is 512 KiB. The 64 KiB comments in `stack.rs`
are stale.

`actor/scheduler.rs:916` turns the caught error into `ExitReason::Error` and calls
`finalize_managed_process`. That path claims finalization once, runs terminate
callbacks, destroys owned secrets, notifies links and monitors, removes the
process, and updates the active count. Terminate callbacks run after resume on
the worker stack, with a separate catch boundary at `scheduler.rs:1142`. They
need the worker's limit and callback-entry protection. A containment test must
observe these effects and a surviving sibling actor. The synthetic probe does not.

`mesh-codegen/src/codegen/mod.rs:379` hoists constant-sized allocas out of body
blocks. It does not reject variable-sized allocas, scalable sizes, over-alignment,
or machine-generated outgoing stack areas. Its object path at line 512 writes
directly to a file. `mesh-repl/src/jit.rs:2646` consumes the module into a separate
MCJIT engine and immediately looks up and calls a function. Both require the same
certification and rejection boundary. The REPL call uses an `extern "C"` function
type today. Safe unwind handling for REPL evaluation is an additional obligation.

Runtime calls are not bounded merely because their entry function is small.
`json.rs:142` recursively serializes nested values. `json.rs:499` recursively
compares nested JSON arrays and objects, including a callback through map equality.
Programmatically constructed values can be nested without relying on the parser's
input limit. Runtime traversal, callback chains, dependency calls, and unwinding
cleanup need explicit bounds or iterative heap worklists. This is a concrete
blocker to an all-runtime stack budget, not an exhaustive runtime audit.

[Native ABI 1](native-packages.md) forbids panic, unwind, and longjmp across native
calls. A pre-call check may fail while still in Mesh. It cannot contain arbitrary
native recursion. Native code and callbacks need a declared, enforced stack bound
with failure caught before returning across the ABI, or isolated execution.

## Conditional implementation contract

Let `A` be the lowest writable stack address. Let `U` bound additional stack use
from detecting failure until control reaches a safe catch boundary. Let `D` bound
the maximum downward excursion from one successful check through the next check.
`D` includes caller stack arguments, ABI call overhead, callee prologue, alignment,
probe instructions and helpers, and any helper used to read the limit.

Continue at a checkpoint only when `SP >= A + U + D`, using checked address
arithmetic. Then the next checkpoint or failure path has at least `U` bytes left,
provided every intervening path obeys `D`. This is an invariant to prove from the
machine code and runtime contracts. Substituting a guessed value for either bound
does not establish it. Initial actor entry, host entry, and callback entry must
establish the same invariant before the first generated prologue.

A feasible implementation must satisfy all these requirements together.

1. Pin the target ABI, LLVM version, optimization pipeline, target features,
   unwind configuration, and allowed instrumentation. Certify the code actually
   published, including late backend transforms and any later LTO.
2. Reject non-entry allocas, nonconstant or scalable allocation sizes, and stack
   realignment outside the certified alignment. Repeat structural validation after
   IR optimization. Disable the red zone for a simple contract. Reject unsupported
   machine stack adjustments, assembly, and target features rather than silently
   accepting them.
3. Obtain a bound on final machine stack excursions, not just the diagnostic's
   fixed frame. Include transient outgoing areas, alignment padding, ABI return
   addresses, and inserted helper calls. A target-specific machine pass that
   validates every stack adjustment can reject unsupported patterns. The existing
   C diagnostic API does not expose `maxCallFrameSize`. Merely parsing the warning
   cannot close that gap. Disabling x86 call-frame optimization is a measured
   candidate restriction that still needs certification in object and JIT paths.
4. Put a non-removable check on every callable generated entry, including leaves,
   closures, actor wrappers, runtime callbacks, exports, and REPL wrappers. Ensure
   inlining, shrink wrapping, tail calls, and outlined code preserve the check-to-
   check bound. A reduction check after a call is too late for the callee prologue.
5. Select the probe method per target and prove its coverage in the emitted code.
   The interval must not exceed the actual guard width. Probes are a last barrier
   against skipping a guard, not the normal containment mechanism. Windows needs
   its ABI-compatible probe and unwind metadata, not the literal `inline-asm`
   attribute used on arm64 and ELF x86 in this experiment.
6. Admit each unchecked runtime call only with enough measured and justified stack
   for its full transitive path. An attacker-controlled tree depth cannot remain an
   unbounded multiplier. Reject, bound, or convert recursive traversals before
   qualifying that call. Apply the same requirement to panic cleanup and finalizers.
7. Raise an ordinary typed unwind while reserve remains. Avoid the general panic
   hook on this path. Catch on the existing actor boundary, restore the worker
   limit on yield, return, and unwind, and finalize once. Keep fault handlers as
   process-fatal fallbacks. Never unwind through a native ABI 1 frame.
8. Collect diagnostics in a context-owned rejection record. Emit to a private
   buffer and publish only after all checks pass. Force MCJIT materialization and
   inspect the same record before constructors, callbacks, or user entry execute.
   Destroy rejected engines. No entry point may use an unchecked alternate path.

No numeric production values for `D` or `U` are established here. A complete
runtime-call inventory, an unwind bound, and a final-machine certificate remain
required before ordinary entry checks are a sound implementation.

## Alternatives and the next proof boundary

A checked pre-prologue entry can compare the incoming SP before any callee frame
allocation. Use a compiler-emitted entry sequence or a target-specific stub that
preserves argument registers and branches to a private body only after admission.
All direct calls, function pointers, exports, closures, and callbacks must name
the checked entry. Calls must never expose the private body. The failure branch
needs correct unwind metadata and enough space for its own bounded operation.
A TLS accessor that pushes a frame before checking defeats the intended ordering.

Such entries avoid relying on a previous check to survive the next prologue.
They still need the body's actual bound, outgoing-call accounting, a runtime-call
policy, and safe failure reserve. They do not by themselves solve unbounded native
or runtime recursion. This unit assesses that design but does not implement or
execute a stub, so its ABI and unwind behavior remain unproved.

Explicit heap continuation frames remove generated-language recursion from the
native stack. They require explicit call and return continuations, GC ownership,
and cleanup state. Runtime and native calls still need a bounded-stack boundary.
That is a larger change than the restricted entry design and is not justified
solely by the fixed-frame warning gaps found here.

Proceed first with a narrowly supported Darwin arm64 machine certificate and a
bounded runtime slice. Before enabling containment, use compiled Mesh subprocess
proofs for direct recursion, indirect recursion, large leaves, nested runtime
values, and near-boundary calls. Observe a surviving sibling, monitor and link
notifications, one finalization, and thread-limit restoration across yield and
panic. Exercise the REPL independently. Run equivalent native proofs before
qualifying Linux or Windows. A target without that proof must report the feature
as unsupported rather than omit protection.

The focused experiment completed. Full workspace tests, authoritative release
proofs, native Linux and Windows execution, actual actor containment, and the
proposed stub and heap designs were not run. The brief explicitly limited this
unit to small probes without another workspace build. No production source,
release state, pull request, or deployment changed.
