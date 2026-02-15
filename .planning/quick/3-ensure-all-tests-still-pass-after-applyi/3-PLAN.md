---
phase: quick-3
plan: 01
type: execute
wave: 1
depends_on: []
files_modified:
  - crates/mesh-codegen/src/codegen/expr.rs
  - crates/mesh-codegen/src/codegen/mod.rs
  - crates/mesh-codegen/src/mir/lower.rs
autonomous: true
must_haves:
  truths:
    - "All cargo test passes across the full workspace (all 11 crates)"
    - "No regressions in 157 e2e tests, 2 compile_fail tests, parser fixtures, or trait_codegen test"
    - "Uncommitted changes in codegen/expr.rs, codegen/mod.rs, and mir/lower.rs are validated"
  artifacts: []
  key_links: []
---

<objective>
Validate that the uncommitted bug fixes in mesh-codegen (expr.rs, mod.rs, lower.rs -- 423 lines changed) do not regress the existing test suite.

Purpose: Ensure the codegen and MIR lowering changes are safe to commit.
Output: Clean test run confirming all tests pass; changes committed if green.
</objective>

<execution_context>
@/Users/sn0w/.claude/get-shit-done/workflows/execute-plan.md
@/Users/sn0w/.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@.planning/STATE.md
@crates/mesh-codegen/src/codegen/expr.rs
@crates/mesh-codegen/src/codegen/mod.rs
@crates/mesh-codegen/src/mir/lower.rs
</context>

<tasks>

<task type="auto">
  <name>Task 1: Run full workspace test suite and verify all tests pass</name>
  <files>crates/mesh-codegen/src/codegen/expr.rs, crates/mesh-codegen/src/codegen/mod.rs, crates/mesh-codegen/src/mir/lower.rs</files>
  <action>
Run the full test suite with `cargo test --workspace` from the project root. This will exercise:
- Unit tests in all 11 workspace crates (mesh-codegen, mesh-typeck, mesh-parser, mesh-lexer, mesh-fmt, meshc, etc.)
- 157 e2e .mpl integration tests in tests/e2e/
- 2 compile_fail tests in tests/compile_fail/
- Parser fixture tests in tests/fixtures/
- tests/trait_codegen.mpl

If ALL tests pass: The bug fixes are validated. Review the diff summary of the 3 changed files to understand what was fixed, then commit the changes with a descriptive message summarizing the bug fixes.

If any tests FAIL: Diagnose the failure(s). Read the failing test file(s) and the relevant changed code. Determine whether the failure is:
  (a) A pre-existing issue unrelated to the changes -- document and proceed
  (b) A regression caused by the changes -- fix the regression in the changed files, re-run `cargo test --workspace` to confirm green, then commit

Do NOT modify test expectations or test files. The goal is to make the existing tests pass with the new code.
  </action>
  <verify>
`cargo test --workspace` exits with code 0 and reports 0 failures. All e2e tests, compile_fail tests, and unit tests pass.
  </verify>
  <done>
Full workspace test suite passes. Changes in expr.rs, mod.rs, and lower.rs are committed with a descriptive message explaining the fixes.
  </done>
</task>

</tasks>

<verification>
- `cargo test --workspace` passes with 0 failures
- `git status` shows the 3 codegen files are committed (no longer in modified state)
- No test files were modified (only the 3 source files)
</verification>

<success_criteria>
All existing tests pass after the codegen/MIR bug fixes. Changes are committed to git.
</success_criteria>

<output>
After completion, create `.planning/quick/3-ensure-all-tests-still-pass-after-applyi/3-SUMMARY.md`
</output>
