# Test tooling implementation evidence

This unit starts at `7a7feda2594593eb6318c17cfb612c8016a99342` in the isolated
`t3-c2ee6d0c` checkout. It covers test selection, bounded asynchronous cases,
and source documentation tests. It does not complete the server runtime
program or implement coverage.

## Implementation choices

`TestPlan` owns case IDs, full names, original byte and line spans, inherited
tags, execution policy, and fixture scopes. The language server keeps the
existing `preprocess_test_source` entry point. Test metadata uses existing
named arguments; no general grammar changes are required.

A file compiles once. The runner supplies selected IDs to that executable.
Consecutive synchronous cases retain one process and declaration order.
Asynchronous cases each get a process, runtime data directory, captured output,
and a completion file. Synchronous groups drain the asynchronous pool before
starting. Completion records and exit status determine success. Printed text
cannot make an early exit pass.

Runnable documentation fences share one compilation at their original module
path. This retains private declarations, imports, test-support helpers, and
source lines without duplicating globally named public declarations. Negative
fences each require a successful control build before a parse or type error
can satisfy the expectation. Documentation examples must be at module scope,
and `# =>` checks a single-line expression. `--doc` rejects test DSL modules.

The existing fixture contract is retained when a scope's own setup fails.
That scope's teardown cannot access bindings whose initialization failed, so
it does not run. Acquired outer scopes still tear down after nested setup
failure. Body and teardown failures are caught per case. Process termination
cannot run Mesh teardown code; the parent reaps the child and removes its data.
There is no new per-case timeout. Explicit external resource paths remain the
application's responsibility.

## Reproduction and commands

At the base, `meshc test --filter chosen` failed with `unexpected argument
'--filter'`. The initial `selects_names_before_running_fixtures` test failed
through the actual CLI with the same error. Logs are retained under
`.tmp/server-runtime/evidence/test-tooling-red.log` and
`test-tooling-red-e2e.log` in this checkout.

All Cargo checks use this environment:

```sh
export PATH=/Users/andreyyurlov/.rustup/toolchains/stable-aarch64-apple-darwin/bin:/opt/homebrew/opt/llvm@21/bin:$PATH
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21
export CARGO_TARGET_DIR="$PWD/target-test-tooling"
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_INCREMENTAL=0
```

Focused commands:

```sh
cargo test --locked -p mesh-parser test_harness --lib
cargo test --locked -p meshc --bin meshc test_runner::tests -- --test-threads=2
cargo test --locked -p meshc --test e2e_test_plan --test tooling_e2e -- --test-threads=2
cargo test --locked -p mesh-lint --test website_examples
cargo test --locked -p mesh-fmt --test corpus every_docs_example_formats_losslessly_and_idempotently
rustfmt --edition 2021 --check --config skip_children=true compiler/mesh-parser/src/test_harness.rs compiler/mesh-rt/src/test.rs compiler/meshc/src/main.rs compiler/meshc/src/test_runner.rs compiler/meshc/tests/e2e_test_plan.rs
git diff --check
```

The process test holds children behind release files. Two children must be
ready together. A third requires one predecessor to finish, and both adjacent
synchronous groups require the appropriate predecessor completions. Separate
actor names and data sentinels verify isolation. A panicking child exercises
teardown and continued execution; an early status-0 exit exercises incomplete
result detection and directory cleanup. These assertions do not infer overlap
from elapsed time.

The CLI tests also exercise duplicate names, inherited and excluded tags,
original-line and fixture-line selection, explicit no-match errors, nested
lexical fixtures, original syntax diagnostics, doctest result failures,
compile-fail expectations, private scope, and test-support helpers.

The new `e2e_test_plan` target is registered in
`.github/workflows/authoritative-verification.yml`. The parent owns integration,
independent review, and the full authoritative workspace and applicable proof
gates. No push, PR, merge, deployment, or release verification is part of this
unit.

## Results

The final CLI run passed all 11 `e2e_test_plan` tests and all 55 existing
`tooling_e2e` tests. The focused parser run passed 11 tests, and the final
runner unit run passed 13 tests. Website example linting and the documentation formatter corpus each passed their test.
`rustfmt --check`, `git diff --check`, and the staged whitespace and sidecar
checks passed. Existing atomic deprecation warnings appeared during builds.

The final CLI and parser logs are
`.tmp/server-runtime/evidence/test-tooling-e2e-final.log` and
`.tmp/server-runtime/evidence/test-tooling-parser-final.log`. The runner unit
log is `test-tooling-runner-final.log` in the same directory. Documentation
checks are in `test-tooling-doc-lint.log` and `test-tooling-doc-fmt.log` in the
same directory. Build output stays in this checkout's private
`target-test-tooling` directory.

The declared execution bound, process isolation, fixture behavior, and CLI
selection are locally verified on macOS arm64. Hosted CI, other operating
systems, full workspace verification, and independent review are not claimed.

## Accepted review corrections

The correction worktree is `t3-cad7dc89` on branch
`t3/fix-test-plan-execution`. It started clean at
`b2352b4c144d986691c4f6c9f9a59b8ffcefc551`. The original feature commit
`e174acc1d5c9b8cb53da3847940a5847411a5d4c` was cherry-picked as
`8f45f99211c771c717b4f23d4d5741e445329ae3` before correction work.

All four accepted review findings reproduced through that worktree's rebuilt
`meshc` CLI before editing. Inputs, commands, statuses, and output are retained
in `.tmp/test-tooling-fix/red.json`. The same fixture replay after correction
is in `.tmp/test-tooling-fix/green.json`.

The fixes establish these contracts:

- A plan distinguishes raw program execution from DSL case execution. Raw
  programs run for directory and explicit-file targets, and their exit status
  determines success without requiring DSL receipts. Case filters skip them.
  Empty files fail compilation because they have no `main`.
- Standalone expectations use lexical comment boundaries. Hashes in strings
  survive. Negative fences leave expectation comments unchanged, retain the
  original `main`, and compile in a separate function. The control build has
  the same wrapper with an empty body. A negative result requires an actual
  parse or type error in the entry module, beyond a failed build status.
- Documentation prefixes never become literal bytes. Padding remains in code
  or moves past a continued literal or into its interpolation. Tests check
  ordinary and triple-quoted multiline values, nested interpolation, and
  diagnostic columns after continued strings.
- Metadata uses the compiler's string value semantics. The existing escape,
  heredoc, and literal-content helpers moved from codegen to the parser's
  `string_content` module. Codegen imports those same helpers. This avoids
  maintaining a separate decoder for names and tags.

The correction uses the environment above with
`CARGO_TARGET_DIR="$PWD/target-test-tooling-fix"`. This private target began as
an APFS copy-on-write clone of the original completed target. All builds ran
in the correction worktree. Neither the original worktree nor the parent
worktree was modified.

The focused checks passed:

- `cargo test --locked -p meshc --test e2e_test_plan --test tooling_e2e -- --test-threads=2`
  passed 19 test-plan tests and 55 existing CLI tests. Eight new CLI tests cover
  the four findings, negative-fence prerequisites and context, and diagnostics.
- `cargo test --locked -p mesh-parser test_harness --lib` passed 14 tests.
- `cargo test --locked -p meshc --bin meshc test_runner::tests -- --test-threads=2`
  passed 13 tests.
- `cargo test --locked -p mesh-codegen --lib string_escapes_are_their_characters`
  passed the shared helper's existing escape test.
- The website example lint and documentation formatter commands above passed.
- `rustfmt --check`, `git diff --check`, and the staged whitespace and sidecar
  checks passed.

Logs are under `.tmp/test-tooling-fix/` in `cli-final.log`, `parser-final.log`,
`runner-final.log`, `strings-final.log`, `doc-lint.log`, and `doc-fmt.log`.
The original fixture replay distinguishes a correctly rejected negative
example from a successful test run. The raw panic changes from status 0 to 1,
the valid negative example changes from status 0 to 1, the multiline example
changes from status 1 to 0, and excluding `network` no longer runs its case.

This is focused macOS arm64 verification. The parent retains fresh artifact
verification, independent fix-diff review, the full authoritative workspace
and proof gates, and integration. There was no delegation, push, PR, merge,
deployment, or release claim in this correction task.
