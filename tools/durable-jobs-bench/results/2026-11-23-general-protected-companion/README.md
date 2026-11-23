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
