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
