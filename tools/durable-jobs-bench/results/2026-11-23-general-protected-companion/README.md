# Generic protected companion stage 04

The controller dispatched the exact stage 04 packet once on November 23,
2026. The clang/lld helper linked and the pinned compiler and runtime built
in 11 minutes 35 seconds. All four ordinary compiler invocations linked
their stock fixtures. The package gate then returned exit 1 because both
positive fixtures were classified on the stock path. The PostgreSQL proof
was never reached. [Controller release](release.json) · [Exact request](request.json)

Both positive native archives declared `__dprintf_chk`. The classifier
recognized `dprintf` and had no entry for its fortified glibc form. An
offline replay of the exact archived symbol tables confirmed that adding
that single import lets both positive archives and the actor archive pass
the import check. The dynamic control still fails that check because it
imports `dlopen` and `dlclose`. The actor closure, final companion links,
and real PostgreSQL effects still require native verification. [Captured classifications](classification-replay.json)

The complete native tar and one exact compiler/runtime ABI pair are under
cleanup hold in the shared checkout. Their lengths, paths, and SHA-256
digests are recorded for a build-free followthrough. The compact packet
preserves the actual exit codes, link/build logs, archive declarations,
native symbol output, and four package receipts. [Retained archive and ABI](retained-native-abi.json)

## Stage 05 cached native proof

The separate stage 05 grant released at 22:42:41 UTC with exit 0. It
reused the stage-04 compiler inputs and runtime ABI, and ran the GNU
relinks and PostgreSQL proofs. The linker audit under `850ec2a7` verified
the actual `__dprintf_chk@GLIBC_2.8` binding from `libc.so.6`.

Alpha and Beta qualified as companion packages. The transitive actor
fixture and the `dlopen`/`dlclose` fixture stayed on stock. The native
proof covers five distinct worker PIDs and exact waits for Alpha, five for
Beta, and one for the dynamic stock control. It records constructor and
thread-local evidence for the companion cases and the expected full-exec
fallback, with independently committed fenced PostgreSQL effects and
ordered terminal settlement. [Alpha](stage-05/proof/alpha/joined-evidence.json) · [Beta](stage-05/proof/beta/joined-evidence.json) · [Dynamic control](stage-05/proof/dynamic/joined-evidence.json)

The compact proof includes raw logs, package receipts, and native phase
exits. The full unmodified tar remains in the shared checkout under cleanup
hold to preserve the exact linked images and maps. [Release](stage-05/release.json) · [Full archive hash](stage-05/raw.json)
