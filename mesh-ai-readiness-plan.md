# Make Mesh easier to develop with an agent

**Decision, 2026-11-20.** On small tasks with clear requirements, Mesh agents completed the write, build, and repair loop unattended after setup. A fresh Codex session completed each of the three tasks below without a developer fixing its code. The biggest avoidable cost was finding the language rule for a specific compiler version. An agent actually searched the compiler executable for syntax. The biggest correctness risk was different: two retry policies could both compile and implement different business rules.

**Build first.** Ship one validated, offline guide with the compiler. Offer to install its skill for the developer’s selected coding tools during interactive CLI installation. Use the same picker later through `meshc skills setup`. Fix the reproducible nested-test panic before promoting the intent recipe. Fix the installer checksum mismatch on the release path in parallel. Then improve the one diagnostic that sent the agent looking through the binary. Reserve language work for demonstrated correctness gaps.

## Baseline on the published compiler

I used the latest GitHub release, `v0.1.8` (2026-09-29, commit `10edaab3`), on macOS arm64. Source recommendations use `7c1620563614a04a573aef806a6d019a2af4dfe9` (2026-11-19). Both report version 0.1.8. I used new `meshc init` projects, separate Git roots, and fresh Codex CLI 0.169.0 sessions with Sol 6.2 xhigh. Sessions had the compiler path, normal host tools and general agent guidance. Baseline projects had no Mesh skill. This was a handful of observed sessions with fixed tasks, not a model success-rate estimate. [Published artifacts](https://github.com/snowdamiz/mesh-lang/releases/tag/v0.1.8) · [Source snapshot](https://github.com/snowdamiz/mesh-lang/tree/7c1620563614a04a573aef806a6d019a2af4dfe9)

| Fresh task | What the agent did without human repair | Independent check |
|---|---|---|
| Write and run a three-string formatter. Pipe the middle value into argument two. | Guessed `\|> format_three("left", _, "right")`. After E0003 it searched strings inside `meshc`, found `\|2>`, and rebuilt. | Ran its executable and compared stdout, including the newline, with `left:middle:right`. |
| Diagnose an incomplete match. Only a timeout should retry. New delivery states must still require a decision. | Read E0012, added the explicit `TimedOut -> true` arm, then deliberately added `Pending` in a disposable copy. | The repaired binary printed `false false true` on separate lines. The extra variant brought E0012 back. The compiler also accepted a wildcard returning false in my counterexample, which printed the wrong third row. |
| Add HTTP 429 to an existing 503 retry policy. Keep the signature, require positive budget for both and reject every other status. | Grouped the allowed statuses before testing the budget. Kept all five original calls and also exercised negative budgets and neighboring statuses. | I rebuilt and compared all five lines to an independently fixed expectation. A deliberately misgrouped alternative compiled and failed the zero-budget assertion. |

The human contribution was the retry policy and the invariants. No developer had to diagnose or patch these three solutions. I supplied tool access, project trust and acceptance criteria up front. I also had to arrange the standalone compiler's runtime archives before the first build. The repair prompt explicitly required future-state coverage, and the repair session read a general debugging skill. The change session ran temporary checks and left no persistent test. These results describe work with stated intent. They do not show that an agent will invent or preserve unstated constraints.

## Small implementation phases

| Order | Developer-visible outcome and affected files | Dependencies | Reproducible acceptance |
|---|---|---|---|
| 1. Publish one usable guide | Convert the 12 nested skills in [tools/skill/mesh](tools/skill/mesh/SKILL.md) into ordinary linked topic references with one valid skill index. Include a verified slot-pipe example. Phase 4 adds a testing topic. Expose the same source through a proposed `meshc guide [topic]` in [meshc](compiler/meshc/src/main.rs), embedded at build time. Add release/source identity or a payload digest to the guide. | Existing skill text, compiler build. | Copy only the built executable to an empty directory, disconnect the network, and read `meshc guide syntax`. Run the slot-pipe example with runtime libraries present. Install a staged copy with `gh skill install ./pack --from-local mesh --scope project --agent codex`. Require one index, all references, valid YAML and correct output. |
| 1a. Repair installer verification | Verify the artifacts people actually download. Accept the release manifest's exact basename with or without a single leading `./` in the [shell](tools/install/install.sh) and [PowerShell](tools/install/install.ps1) readers. Keep the public copies in sync. Document `bin/` and `lib/` layout in the [tooling guide](website/docs/docs/tooling/index.md). | Independent of phase 1. | Feed the **published-format** manifest into the staged Unix and Windows [installer proof](scripts/verify-m034-s03.sh). In strict mode both archives must verify, both CLIs must report the pinned version, and the installed compiler must build and run the fixture. A changed byte must fail. Exercise the final merge-of-assets job in [release.yml](.github/workflows/release.yml), which emits a different spelling from the per-target stage. |
| 1b. Run nested acceptance tests | Compile nested checks that contain a local binding. Trace comprehension body typing through MIR and [list conversion](compiler/mesh-codegen/src/codegen/expr.rs). | Independent, required by phase 4. | Use the reduced nested test below on release and HEAD. Replace exit 101 with one named pass. An intentionally wrong assertion must exit 1. |
| 2. Choose coding tools and scope | Add `meshc skills setup` to [meshc](compiler/meshc/src/main.rs). List Codex, Claude Code, Cursor, Gemini CLI and GitHub Copilot CLI. Let the user select multiple tools and choose account or project scope. Preview the resolved directories and write the embedded skill once per distinct destination. | Phase 1. | Run setup offline from a standalone executable in an isolated home and project. Select Codex, Cursor and Claude Code at project scope. Check the shared directory and the Claude directory, then run task A in a fresh host session. Repeat at account scope using the host-specific paths below. Declining must leave all destinations untouched. |
| 2a. Offer setup during installation and preserve it across upgrades | Have the [Unix](tools/install/install.sh) and [Windows](tools/install/install.ps1) installers call the installed executable’s picker after interactive installation. Update their website copies and the [tooling guide](website/docs/docs/tooling/index.md). Keep host detection and file ownership in the CLI. | Phase 2. | Exercise the published shell and PowerShell installation commands with an interactive terminal. Accept once and decline once. Run unattended with no terminal and with the installer’s accept-defaults flag. Verify fresh unattended installs perform zero skill writes. Upgrade an enrolled account with clean, edited and pinned fixtures. Check their contents as described below. |
| 3. Repair the pipe diagnostic | For the demonstrated pipe-plus-placeholder mistake, explain the implicit argument and show the slot syntax. Use [piped call inference](compiler/mesh-typeck/src/infer.rs), [error representation](compiler/mesh-typeck/src/error.rs) and the [shared renderer](compiler/mesh-typeck/src/diagnostics.rs). Expose the same code in the [LSP adapter](compiler/mesh-lsp/src/analysis.rs). | Phase 1 for the offline topic link. Can develop against the baseline fixture independently. | Run the exact failing source below with and without `--json`. Retain E0003, stderr JSONL, byte spans and existing keys. Assert a relevant hint for this case, ordinary arity text for ordinary calls, and a UTF-16 range plus code over LSP. Repair and assert the exact string order. Extend [rendering tests](compiler/mesh-typeck/tests/rendering.rs), [CLI fixtures](compiler/meshc/tests/e2e_language_fixes.rs) and the [LSP wire test](compiler/mesh-lsp/tests/server.rs). |
| 3a. Explain the wildcard trade-off | Change E0012 help in [diagnostics.rs](compiler/mesh-typeck/src/diagnostics.rs) to name the missing arm and explain that a wildcard also covers future variants. Keep wildcards legal for intentional defaults. | Independent. | Use the task B fixture. Terminal and JSON identify `TimedOut`. Test today's three outputs and then add `Pending`: only the explicit implementation requires another arm. |
| 4. Verify an agent’s change unattended | Add a worked retry example to the guide and [testing docs](website/docs/docs/testing/index.md). Show the agent how to derive a regression test from a request and existing behavior, exercise it with `meshc test`, and report the result. | Phase 1 and the nested-test fix below. | Give a fresh agent the original task C project and only the request to add 429 under the existing budget rule. With zero follow-up prompts, it must leave a runnable test, pass the five-row executable check, and catch a deliberate regression. The evaluator runs its own held-out budget cases. |
| Pilot. Verify a pure retry policy | Give an agent a compiler-checked requirement and a reproducible counterexample when the implementation violates it. See [law placement and verification](#law-placement-and-verification) for the file layout, ownership rules and phased checks. | Independent of the guide and installer. Start with the existing retry fixture. | The native program and finite budget sweep pass a policy capped at eight. The proposed verifier must reject it, give an input that fails on the compiled function, and prove the restored policy. |

**First install.** Once the CLI is usable, ask “Set up the Mesh skill for your coding tools? [y/N]”. On yes, show the tool and scope picker. Display the actual project directory when offering project scope. For example, choosing Claude Code and Cursor at project scope installs the skill under `.claude/skills/mesh/` and `.agents/skills/mesh/`. If Codex is also selected, its shared directory receives one copy. Explain that other hosts watching that directory can also discover the skill. Use host-specific personal paths for account scope. At any time, `meshc skills setup` runs the same flow from the compiler currently on PATH, including a manually downloaded or source-built binary.

**Terminal handling.** The documented Unix install pipes a script to `sh`. Read interactive answers from the controlling terminal so the shell’s script stream stays intact. Use the interactive console on Windows. A fresh CI install, an installer invoked with `--yes` or `-Yes`, or a declined prompt finishes with the ordinary CLI and prints the optional setup command. Host trust and tool permissions remain with the host.

**Upgrades.** Record which destinations Mesh installed and their payload digest. Refresh an enrolled account copy only when its files still match the previous installed tree. Keep project copies pinned until setup is explicitly rerun there. If the destination contains edits, extra files, an unfamiliar skill or a symlink, preserve it and report the path. Stage writes so interruption leaves a usable old or new copy. Verify repeated setup at the same version, recovery after interruption and two projects pinned to different compiler builds. Reading `meshc guide` always uses the running binary’s embedded content.

For implementation changes, use the affected jobs in [authoritative verification](.github/workflows/authoritative-verification.yml) and the [Unix and Windows release proofs](.github/workflows/release.yml). Run the documentation build for site changes. Run the real program and the deliberate failing case alongside the Rust checks.

## What a fresh installation actually provides

The macOS compiler archive contained exactly `meshc`, `libmesh_rt.a` and `libmesh_test_rt.a`. I verified its SHA-256 against the published manifest. Extracting all three side by side and building failed with “Could not locate Mesh runtime static library”. Moving the compiler to `<prefix>/bin` and the archives to `<prefix>/lib` fixed it. The release offers separate compiler and package-manager archives. Someone copying an executable out of an archive never runs a Mesh installer. [Runtime lookup](compiler/mesh-codegen/src/link.rs) · [Release assets](https://github.com/snowdamiz/mesh-lang/releases/tag/v0.1.8)

I ran the checkout's Unix installer against v0.1.8 in an empty home. Normal mode installed both tools, their libraries, version and shell setup, with no skill. It warned twice that it skipped checksum verification. Strict mode stopped before installing the compiler. The repo’s own Unix installer proof, run with the public release archives as prebuilt inputs, failed at the same install step. The manifest lists `./meshc-…tar.gz`, but the reader compares the second field to the bare filename. PowerShell has a corresponding filename match; only the Unix failure was run here. Both installers skip a matching installed version. Both CLI update commands delegate to the installer; Windows waits for the running process before replacing it. An unattended update's `--yes` cannot stand in for permission to write guidance into unrelated projects. [Updater](compiler/mesh-pkg/src/toolchain_update.rs) · [Published installer sources](website/docs/public/install.sh) · [Windows source](website/docs/public/install.ps1)

**Challenge to embedding the guide.** A tagged GitHub skill would cost less to ship. I used `gh skill install snowdamiz/mesh-lang tools/skill/mesh/SKILL.md --pin v0.1.8 --scope project --agent codex`. GitHub CLI 2.102.0 could preview the tagged Mesh index, but a real install failed for both Codex and Claude Code and left a partial tree. The nested actors description contains an unquoted colon followed by a space. `gh` reported `could not inject metadata: invalid frontmatter YAML: yaml: line 2: mapping values are not allowed in this context`. I converted the topic files to `references/*.md`, retained one index and updated its paths in a disposable copy. Its local install succeeded. A second install over a locally edited index refused with exit 1, and the edited index and an extra user file kept their hashes. Pinned GitHub distribution is a viable companion once the layout is fixed. It still requires another tool, a source pin and usually a network connection. The binary already identifies the exact guide it was built with, even when source and release share a version string. [GitHub CLI install](https://cli.github.com/manual/gh_skill_install) · [Pin and overwrite behavior](https://cli.github.com/manual/gh_skill_update)

### Host support and consent

Official documentation checked 2026-11-20. The proposed picker offers account scope for use across projects and project scope for a repository-specific pin. Merely putting Markdown in a compiler installation directory did not enroll the sample project. The table distinguishes host discovery from a model actually applying the guide.

| Host | Where and when it loads guidance | Consent and live result |
|---|---|---|
| Codex | Searches `.agents/skills/` from the working directory through the repo root, plus `~/.agents/skills/`. Metadata is available first. A relevant request or explicit `$mesh` can load the body. | I trusted the disposable project. The corrected package installed through `gh`. On the same formatter prompt as the control, Codex read the index and `references/syntax.md`, used `\|2>`, and passed on the first build. No explicit skill invocation. One paired task demonstrates usefulness, not an effect size. [OpenAI skills](https://learn.chatgpt.com/docs/build-skills#where-codex-loads-local-skills) |
| Claude Code | Project `.claude/skills/`, personal `~/.claude/skills/`. Description-based selection or `/mesh`. Personal skills can shadow project skills. | Interactive workspace trust and tool approvals apply. Claude Code 2.1.325 listed the staged skill. The default profile required login. With the configured authenticated profile, Fable 5.2 invoked `Skill(mesh)`, read `references/syntax.md`, built the formatter and printed the expected string. I reran its binary. [Skills](https://code.claude.com/docs/en/skills) · [Trust](https://code.claude.com/docs/en/security) |
| Cursor | Project `.agents/skills/` or `.cursor/skills/`, corresponding home directories and compatibility paths. Recursive discovery at startup, then relevance or `/mesh`. | Optional workspace trust and separate command approvals. Personal-skill cloud sync needs separate confirmation. The staged project install selected `.agents/skills/`. Cursor Agent reported “Not logged in”. Host activation remains unverified. [Skills](https://cursor.com/docs/skills) · [Security](https://cursor.com/docs/agent/security) |
| Gemini CLI | `.gemini/skills/` or `.agents/skills/` at workspace or user scope. Scans metadata on startup or `/skills reload`; the model requests `activate_skill`. | Trust the workspace to discover its skills. Activation prompts before loading instructions/resources under the default policy. In 0.70.0, `gemini skills list` went from no skills to `mesh [Enabled]` after trust. The model request required authentication. [Discovery](https://geminicli.com/docs/cli/skills/) · [Activation consent](https://geminicli.com/docs/cli/using-agent-skills/) |
| GitHub Copilot | CLI searches project `.github/skills/`, `.agents/skills/` or `.claude/skills/`, and personal `~/.copilot/skills/` or `~/.agents/skills/`. Model selection by relevance, explicit `/mesh`, refresh with `/skills reload`. Commit project guidance for the GitHub-hosted agent. Use the documented `.github/skills/` path for Copilot in VS Code. | CLI directory trust, tool approval and organization policies still apply. Copilot CLI 1.0.114 listed the staged project skill. The isolated profile lacked authentication. With existing GitHub credentials the hosted request returned “Access denied by policy settings”. [CLI skills](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills) · [CLI trust](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/configure-copilot-cli) · [Supported clients](https://docs.github.com/en/copilot/concepts/customizations/agent-skills) · [VS Code](https://code.visualstudio.com/docs/agent-customization/agent-skills) |

The picker shows the install destinations before writing. Reading a skill, running tools and syncing personal data each remain subject to the host's own rules. Keep the files instruction-only, use explicit relative links and drop the current index's promise that it “Auto-loads for all Mesh-related questions.” For hosted agents, a local install reaches them only when the files are in their checkout or provisioned during setup.

## A compiler error worth changing

This is the first program the unaided formatter session tried. Save it verbatim as `main.mpl`.

```mesh
fn format_three(left :: String, middle :: String, right :: String) do
  "${left}:${middle}:${right}"
end

fn main() do
  let formatted = "middle" |> format_three("left", _, "right")
  println(formatted)
end
```

Actual v0.1.8 stderr from `meshc build . --no-color`, exit 1:

```text
[E0003] Error: arity mismatch: expected 3 arguments, found 4
   ╭─[ ./main.mpl:6:31 ]
   │
 6 │   let formatted = "middle" |> format_three("left", _, "right")
   │                               ────────────────┬───────────────
   │                                               ╰───────────────── expected 3 argument(s)
   │
   │ Help: 1 extra argument(s)
───╯
error: Compilation failed due to errors above.
```

`meshc build . --json` also exited 1. Stdout was empty; stderr was exactly:

```jsonl
{"code":"E0003","severity":"error","message":"arity mismatch: expected 3 arguments, found 4","file":"./main.mpl","spans":[{"start":149,"end":181,"label":"expected 3 argument(s)"}],"fix":"1 extra argument(s)"}
{"code":"C0001","file":"","fix":null,"message":"Compilation failed due to errors above.","severity":"error","spans":[]}
```

Proposed terminal output for the same site:

```text
[E0003] ./main.mpl:6:31: format_three takes 3 arguments. This pipe supplies 4.
  |> passes "middle" as the first argument; _ does not select a slot.
  To pass it as argument 2, write: "middle" |2> format_three("left", "right")
  More examples: meshc guide syntax
```

With that replacement the program printed exactly `left:middle:right`. Removing the underscore alone might satisfy the arity checker while passing the three strings in the wrong order. **Limit the suggestion to the recognizable call shape.** The existing E0044 help already describes slot syntax. A two-argument control, `1 |> pair(_)`, had matching arity and produced E0004 for the unbound underscore. Keep that distinct when testing the proposed E0003 change. E0003 also covers ordinary arity errors and tuple destructuring. Treat advice as text. Add machine-applicable edits only for verified, unambiguous replacements with a file, a range, replacement text and applicability. Keep existing JSON consumers working. The current `fix` contains prose. LSP already translates ranges to UTF-16 but drops diagnostic codes and help. [Mesh renderers](compiler/mesh-typeck/src/diagnostics.rs) · [LSP conversion](compiler/mesh-lsp/src/analysis.rs)

Other observed errors already supported a repair. Changing the integer literal to a string gave ``[E0001] Error: type mismatch: expected `Int`, found `String` `` with the label `argument 1 has type String, expected Int`; `fix` was null. Misspelling `retry` gave ``[E0004] Error: undefined variable `retri` `` and ``Help: did you mean `retry`?``. I corrected each and checked the original five outputs. E0012 gave the missing variant by name and suggested explicit patterns or a wildcard. The product requirement determined which repair was correct. The follow-up below addresses E0012’s wildcard advice. A catalog-wide rewrite has little support in this sample.

**What transfers from Solid 2.** Its stable 2.0.0 release shipped on 2026-11-17. In development, a reactive runtime finding can name the code, observed behavior, owner and location. The first report for a code links its human explanation and the matching section of a repair skill shipped with the package. Observe builds expose events, and tests can assert on captured diagnostics. Mesh already has source spans, codes and JSONL. Borrow the short explanation plus a version-matched local topic, and test that links resolve. Solid's owner graph and runtime instrumentation solve a different problem. Printing a path or command cannot activate a skill in any portable way. In a plain terminal the user sees a repair, can type the guide command, and can rebuild offline. An agent host may select a skill, request consent, follow the reference or ignore it. [Solid 2 release](https://github.com/solidjs/solid/releases/tag/solid-js%402.0.0) · [Diagnostic anatomy](https://docs.solidjs.com/reference/solid-js/diagnostics/) · [Shipped repair skill](https://github.com/solidjs/solid/blob/a8186e6738b3a6a3341c561f66e9325432d6759b/packages/solid/skills/reactivity-diagnostics/SKILL.md) · [Catalog consistency test](https://github.com/solidjs/solid/blob/a8186e6738b3a6a3341c561f66e9325432d6759b/packages/solid/test/diagnostics-catalog.spec.ts)

## What to borrow from Bend

The match is **Victor Taelin's Bend 2**. The project's own site explicitly connects spending less time reading or writing code with stating precise laws for an AI to implement. The guide separates human-reviewable `LAWS.bend` from implementation and `PROOF.bend`. The guide says `bend PROOF.bend` requires the neighboring laws to be imported, and `bend PROOF.bend --verdict` rechecks proofs with a Lean-verified kernel and excludes unsafe or foreign dependencies. The guarantee depends on the law expressing the right requirement, on the translation, and on the execution backend. The older HVM2-based parallel language is a common source of confusion. Its repository is now Bend1; search results for the original Bend URL can redirect to the new project. The attributable claim is a development method and a stated ambition. It does not establish that a prose request uniquely specifies correct software. [Project statement](https://bend-lang.org/) · [Bend guide](https://github.com/bendlang/bend/blob/main/guide/GUIDE.md#laws-and-proofs) · [Bend 2.0.40, 2026-11-06](https://github.com/bendlang/bend/releases/tag/v2.0.40) · [Bend 1 archive](https://github.com/HigherOrderCO/Bend1)

**Walkthrough.** Start with “retry a 503 only if there is budget left.” Use arguments `(503,1), (503,0), (429,1), (429,0), (500,1)`. The agent can derive `true,false,false,false,false` from that request and check it against the original binary. Change the request to include 429 under the same budget constraint. Only the third result changes. Keep the other four as regression checks. The reference implementation and the plausible mistake differ by one pair of parentheses:

```mesh
pub fn retry(status :: Int, left :: Int) -> Bool do
  (status == 503 || status == 429) && left > 0
end
```

With the parentheses removed, the native executable returned true for `retry(503, 0)`. The named Mesh test failed on precisely that assertion. Conversely, changing `left > 0` to `left == 1` **passed all five examples**. A second test iterating HTTP statuses 100 through 599 and budgets −1, 0, 1, 2 and 8 rejected it at budget 2. Both tests passed on the intended implementation. This is evidence about a finite domain. A Bend-style universal law would quantify over *every* supported budget. Even a proof of that law would say nothing about an omitted requirement such as idempotency of the operation being retried.

Keep the signature and exhaustive variants in the **language**. Put example projects, nonempty test discovery and reproducible diagnostics in **tooling**. In the **agent workflow**, the initial request sets the goal. The agent works out boundary cases from the request, existing code and tests, implements the change, and verifies the result. It can report any assumptions alongside the test output. Existing `meshc test` supports imported production code. `meshc proof` runs named repository scenarios. Before funding a general contract language, measure real cases where a build and visible tests pass but an independently specified check fails. Trial a verifier on one such pure module and compare what it catches and costs against a bounded reference model. [Test runner](compiler/meshc/src/test_runner.rs) · [Scenario proofs](compiler/meshc/src/proof.rs) · [Existing SQLite behavior checks](compiler/meshc/tests/e2e_sqlite_todo.rs)

### Independent opinions changed the plan

**Claude Fable 5.2** identified a requirement the five output rows cannot express: adding a new delivery variant should force another decision. I checked the objection. Replacing the explicit timeout arm with `_ -> true` preserved *all three* current outputs. Adding `Pending` still built and still passed those checks. With the explicit arm, the same addition failed with E0012. Fable also pointed to Bend's own HTTP body example: its authors explain that an implementation which merely strips an initial blank line can satisfy both stated laws. A checked law is only as complete as the behavior its reviewer specified. [Bend HTTP laws](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/demos/io_http_fetch/LAWS.bend) · [Bend guide at reviewed commit](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/guide/GUIDE.md#laws-and-proofs)

**Gemini 4 Neon xhigh** argued for repeatable properties and a gate that verifies the intended specification actually ran. Its next candidate was Mesh's binary reader: a sequence of successful reads should advance the offset by the bytes consumed and stay within the input. Agreement on five individual calls would say little about a longer sequence. I confirmed the underlying gate problem: a fresh project with no test files exited 0, and a named test whose entire body was `0` also passed. Neon would invest in seeded generators and shrinking sooner than Fable, who wanted measured failures across real maintenance tasks first. [Mesh binary reader](packages/mesh-binary/binary/reader.mpl) · [Bend proof-gate discussion](https://github.com/bendlang/bend/issues/776)

**Decision for Mesh.** Make autonomous development the default. For “handle 429 like 503,” the existing 503 policy already answers the budget question. The agent can write the regression test, run it against the old and new implementations, and finish in one session. Explicit match arms can preserve constraints on future variants. Tests can preserve constraints on values and sequences. A team can supply protected acceptance checks or request review for particular changes. Ask for a product decision only when the request and repository leave a consequential choice unresolved and the project has given the agent no authority or default for deciding it.

I reran task C with the reviewed budget test already in the project. It failed on the original 503-only implementation. A fresh Codex session changed the policy, preserved the five calls, and passed the same test. I independently rebuilt, checked stdout, and verified the test SHA-256 remained `f47c1bdf10ca9f52f3003549a840a0d23d96a620fc750dafc82fa216aadeb238`. This additional run shows that an unattended agent can also honor a team-supplied check. Phase 4 separately evaluates whether the agent leaves a useful test when given the request alone.

**Why pilot proof syntax.** A source-level law could give an autonomous agent a stronger completion check. I modeled the retry predicate and its intended behavior separately in Z3 with signed 64-bit comparisons, matching the [compiler’s Int width](compiler/mesh-codegen/src/codegen/types.rs). Asking for an input on which the two disagreed returned `unsat`. Adding `left <= 8` to the implementation passed every status and budget in the earlier sweep. Z3 found a disagreement, and I confirmed `status = 429, left = 9` is a witness. This verifies the handwritten model. The next experiment must check actual Mesh source so a stale or incorrect translation cannot silently stand in for the implementation.

### Law placement and verification

**Pilot decision, revised 2026-11-20.** Allow a team to keep all its laws in `laws/`, beside the source, or in a directory it chooses. A law imports its target by Mesh module and function name. Its filename and parent directory have no part in binding. Use `laws/` in the starter example. A project with its laws there or alongside source needs no configuration.

```text
retry-service/
  mesh.toml
  policy.mpl                 # defines pub fn retry
  laws/http/eligibility.law  # imports Policy.retry
  tests/budget.test.mpl      # runs concrete examples
```

Proposed `laws/http/eligibility.law`:

```mesh
from Policy import retry

law retry(status :: Int, left :: Int) -> Bool do
  case status do
    429 -> left > 0
    503 -> left > 0
    _ -> false
  end
end
```

The first line uses familiar Mesh import syntax. The `law` header names the imported function. For every pair of Mesh integers, the verifier calls the modeled function and asks whether its result can differ from the table. Parameters match by position and type. This form keeps the expected expression independent of the implementation. Allow several targets in a file and one complete result law per target in the pilot. Reporting the resolved target in every verdict makes it clear what was checked. A law comparing two functions, such as decode after encode, would motivate a later named property with explicit calls.

**Discovery.** By default, discover visible `.law` files recursively in the selected package, including `tests/`. Nested packages, dependency checkouts, hidden directories and build output are outside the default scan. A team that wants to select directories can add the following proposed section to `mesh.toml`. Paths are relative to the package root, including when verification is invoked from another working directory. Each listed directory is scanned recursively. This setting selects the law files and has no effect on Mesh module lookup.

```toml
[verify]
law_roots = ["laws", "specs/contracts"]
```

Explicit roots replace the default scan. Reject missing roots, an empty list, paths outside the package, nested-package roots and symlink traversal. Overlapping roots count the same file once. Distinct files claiming the same resolved target are an error with both source locations. `meshc verify . --list` should show the effective roots, paths and fully qualified targets. A narrowly configured run can omit files outside its roots, which is why CI also compares the trusted inventory described below.

**Visibility.** For a public target, use `from Policy import retry` at any location. For the [private baseline function](#seed-the-original-retry-policy), propose a law-only form, `from Main import private retry`. Restrict it to functions defined in the law’s own package. It can select a proof target without exporting the function to ordinary Mesh code or permitting access to a dependency’s internals. Continue to enforce E0035 on ordinary private imports. An ordinary `.mpl` file cannot use the new form. For an overridden entrypoint at `lib/start.mpl`, use the actual module name `Lib.Start`. A root `main.mpl` still names `Main`. Avoid an extra entrypoint alias until a real task needs one.

| Alternative | What a developer gets | Decision |
|---|---|---|
| Infer the target from the matching sibling filename | Short declaration and private access. Moving the law changes or breaks the binding. | Keep adjacent files as a layout choice. Require an explicit import there too. |
| Import only public functions everywhere | One existing access rule. Covers public policies and the private helpers they call. | Implement first. Add the explicit private-target form for the private baseline. |
| Configurable search directories plus explicit imports | A centralized or mixed layout, one binding rule, stable targets after law-file moves. | Adopt. Scope discovery and binding to the package. Reject ambiguity instead of choosing the first file in search order. |

OpenJML documents separate specification search paths and first-match precedence. Move permits a specification module separate from its implementation. Both show how projects can organize requirements independently of source layout. Mesh can give each law an explicit target and diagnose duplicates so search order cannot change the result. Sources checked 2026-11-20. [OpenJML specification paths](https://www.openjml.org/tutorial/SpecificationFiles) · [Move specification modules](https://github.com/move-language/move/blob/main/language/move-prover/doc/user/spec-lang.md#separating-specifications)

**Release experiment.** On the checksum-verified v0.1.8 macOS arm64 archive, I created a fresh package with a private `Policy.retry` and a public wrapper. An unreferenced `contracts/retry.mpl` importing the private function failed the build with E0035. Importing the wrapper succeeded, including with an ordinary function named `law`. Renaming this valid module to `policy.law.mpl` still built. Replacing it with proposed law syntax caused a parse error at line 1. The identical text as `policy.law` was ignored by the build. Under `tests/policy.law.mpl`, both build and test exited zero and the latter printed `No *.test.mpl files found.` This justifies a distinct extension, explicit discovery and a verification command with its own success criteria. Today’s source publisher also filters by the final `.mpl` extension. [Source discovery](compiler/mesh-pkg/src/project.rs) · [Test discovery](compiler/meshc/src/test_runner.rs) · [Package archive members](compiler/meshpkg/src/publish.rs)

**Import experiment, 2026-11-20.** With the same verified release, I ran a named test as `retry.test.mpl`, `laws/transport.test.mpl` and `specs/nested/unrelated_name.test.mpl`. Its bytes stayed identical. All three passed with `from Policy import retry`. Reversing the budget condition failed the centralized test at `(429, 9)`. Making the target private gave E0035. Changing the import to `FormerPolicy` gave E0031. Reproduce with a clean project: export the [grouped policy](#what-to-borrow-from-bend) in `policy.mpl`, place the [named budget test](#run-the-independent-budget-check) at each path in turn and run `meshc test <path>`. These runs exercise actual Mesh imports and finite assertions. The `.law` parser, private selection and universal verification remain proposed.

**Review evidence.** Fable 5.2 proposed public imports. Neon xhigh proposed a dedicated laws directory. Astra 6.2 identified the private baseline and existing `.law.mpl` compatibility issue. The previous sibling-only rule bundled target identity with storage. Explicit imports address both layouts. Sol 6.2 found that the import context keys exports by the last module segment, and that private exports carry names for diagnostics without full signatures. For the pilot, limit direct targets to the selected package. Record their actual owning package and checked declaration. A target may call an imported helper if the verifier can inspect and model its body. Reject ambiguous module names and keep the existing duplicate-public-function diagnostics. [Import context](compiler/mesh-pkg/src/project.rs) · [Exported and private names](compiler/mesh-typeck/src/lib.rs) · [CLI duplicate checks](compiler/meshc/src/main.rs)

**Resolver regression reproduced on v0.1.8.** Create `a/policy.mpl` with public `first() -> Int` returning 111 and `b/policy.mpl` with public `second() -> Int` returning 222. In `main.mpl`, `from A.Policy import first` alone built and printed 111. Adding `from B.Policy import second` gave E0034 claiming that `A.Policy` exported `second` and lacked `first`. In a separate project where `lib/start.mpl` was the entrypoint, importing a public value from the root `Main` printed 71. Making that value private gave E0035. Full module identity matters to ordinary application code as well as to laws.

#### Implementation and acceptance

Each row is a reviewable change on the pilot branch. The command and grammar below are proposals. The release experiment above used only shipped commands.

| Phase | Developer-visible outcome and affected tooling | Depends on | Reproducible acceptance |
|---|---|---|---|
| L0. Resolve dotted imports consistently | Key selective imports by the full module path in the [project loader](compiler/mesh-pkg/src/project.rs) and [import inference](compiler/mesh-typeck/src/infer.rs). Share that lookup with law binding. Keep source-level shorthand where unambiguous. | Independent compiler fix. | Build the two-`Policy` fixture above, call both functions and require stdout `111:222` in both import orders. The existing error for genuinely duplicate public symbols must still fire. |
| L1. Discover and bind public targets | Add `meshc verify . --list`. Implement shared law discovery in [mesh-pkg](compiler/mesh-pkg/src/project.rs), optional roots in the [manifest](compiler/mesh-pkg/src/manifest.rs), and imports in a dedicated [law parser](compiler/mesh-parser/src/lib.rs). Bind by resolved owning package, full module name, function and arity; validate the typed signature. Keep law imports outside the executable import graph. | L0, the existing project type checker and the CLI’s build checks. No solver. | Move the example from `policy.law` to `laws/http/eligibility.law` to `specs/contracts/budget.law`. With each location selected, list the same target, signature and one law. Test default, custom and overlapping roots, package boundaries and malformed paths. Reject missing imports, ambiguous modules, mismatched signatures, duplicates, trailing junk, empty files and zero laws. Exercise two modules ending in `Policy` with distinct public functions. |
| L1a. Bind an explicitly private target | Add `from Module import private name` to the law parser. Read the selected package’s checked declaration and module ID directly. Report candidates on a failed or ambiguous binding. Ordinary exports and runtime linkage keep their present visibility. | L1. | A law in `laws/` binds the unused private `Main.retry`. With a configured `lib/start.mpl` and a distinct root `main.mpl`, select the intended function in each by its full module name. An ordinary Mesh import still fails with E0035. An attempt to select a dependency’s private function or to use this syntax in application source fails. |
| L2. Prove or return an input | Add the solver path to `meshc verify .`. Derive the actual function and resolved direct callees from [typed per-module MIR before reachability pruning](compiler/mesh-codegen/src/lib.rs). Encode signed 64-bit comparisons and the actual Boolean and conditional semantics. Emit `verified`, `falsified` or `unverified` per bound law. Identify unsupported constructs at their source spans. | L1 and a pinned solver. | Run the law above against the grouped policy. Expect one verified law and exit 0. Append `&& left <= 8` to the grouped expression. The named finite test still passes. Verification must exit nonzero with a witness. Replay it on the compiled policy, compare expected and actual, then restore and reverify. Separately try a private helper that reads the clock and discards the result, a recursive call, division, a missing solver and an exhausted solver budget. Each must exit nonzero and explain why the claim is unverified. Run ordinary build and test with the solver absent. |
| L3. Preserve requirements across moves and release archives | Use the same manifest and sorted inventory in `verify --list`, verification and the [publisher](compiler/meshpkg/src/publish.rs). Emit the law’s resolved identity, normalized requirement digest and diagnostic path. Provide the trusted-base comparison below, including machine-readable results. | L1–L2. The CI job supplies the trusted baseline. | Publish and unpack a fixture with custom roots and compare inventories. With two laws, move and rename one and update the root configuration: both targets still verify. Delete one, weaken it, redirect its import, or narrow the roots to hide it: an implementation-only job must fail. On an authorized request to add 429, the agent can change the designated requirement, observe a failure on the old code, update the policy, and pass verification and the independent budget test unattended. |

For L2, solve for an input where the implementation and the expected expression differ. An unsatisfiable query verifies the law over the modeled input domain. One valid counterexample is `status = 429, left = 9, expected = true, actual = false`. Accept any witness that reproduces. For the private baseline target, replay it by adding the reported call to the disposable fixture’s `main` in the same module. Inspect direct private and imported callees, including discarded expressions, before optimization. Mesh’s plaintext analysis permits clock and random modules because it checks data disclosure. The verifier needs its own explicit list of deterministic, terminating operations. Keep preconditions, axioms and unchecked external calls out of the pilot. A parse failure, a native panic or a solver returning `unknown` proves no law. Verification exits zero only when at least one law ran and every law verified. [Mesh integer representation](compiler/mesh-codegen/src/codegen/types.rs) · [Plaintext analysis](compiler/mesh-typeck/src/plaintext.rs)

**Native replay already run on v0.1.8.** To repeat today, copy the decision table into `main.mpl` as an ordinary `fn expected(status :: Int, left :: Int) -> Bool` and import `retry` once from `policy.mpl`, and print both results for `(429, 9)`. Run `meshc build . && ./output`, then `meshc test tests/budget.test.mpl` with the [budget fixture](#run-the-independent-budget-check). The correct function printed `true true`; the capped function printed `true false`. The named budget suite passed for both. I also got that disagreement at `(503, 9223372036854775807)`. This checks the proposed table and witnesses on native Mesh. A source-integrated verifier remains the L2 deliverable.

**CI and file moves.** Replace the earlier path-based Git check: it flags a pure rename and misses a `mesh.toml`-only change that excludes a law while another still runs. An implementation-only task should preserve every previously checked requirement even when it moves. Have trusted CI run the same verifier on a clean base checkout and the candidate. Store the base output outside the candidate’s writable tree. Proposed commands:

```sh
set -e
meshc verify "$BASE_CHECKOUT" --list --json > "$TRUSTED_ARTIFACTS/base-laws.json"
meshc verify "$CANDIDATE_CHECKOUT" --preserve "$TRUSTED_ARTIFACTS/base-laws.json"
```

For this pilot, identify a requirement by its owning package, canonical module, target name and typed signature. Compare its normalized, resolved law expression, including the access mode. Exclude the law filename, line numbers, comments and whitespace. Show old and new paths in the report. A missing identity or changed requirement fails preservation; newly added laws must also verify. Moving the implementation to another module changes its target identity and needs an explicit migration. For a requested behavior change, the job can allow that target’s requirement to change and still check all other requirements. No per-edit approval is implied. The existing [report comparison](compiler/meshc/src/plaintext_report.rs) is a local precedent.

I exercised the inventory rule with a disposable two-target model. A move and rename preserved the entries. Narrowing roots lost one target, redirecting an import lost the original target, and changing the expected expression changed its digest. The model used fixed signatures and a body hash. Compiler-resolved imports, normalized syntax and the package archive round trip are L1–L3 acceptance checks.

**Decision after the pilot.** Advance only if a fresh agent can repair the capped policy using the verifier output, preserve an existing law through a refactor, and complete the authorized 429 requirement change unattended. Before reserving broader syntax, run the same exercise on a second real policy. For the [binary reader](packages/mesh-binary/binary/reader.mpl), successful reads should advance by the number of bytes consumed. Its records, `Bytes`, arithmetic and `Result` exceed L2. Record an honest `unverified` and compare the cost of supporting those operations with an executable sequence test. A proof-term language would need a useful obligation the automatic checker cannot settle and an independently checkable proof an agent can maintain. Keep HTTP, database and scheduler behavior covered by integration tests.

### Fix the compiler failure exposed by the acceptance test

A natural first version nested the budget loop inside the HTTP-status loop. Both v0.1.8 and a fresh `cargo build --workspace --locked --offline` of `7c162056` panicked during code generation, before any assertion ran. Here is the reduced `tests/nested.test.mpl`. Removing the single `let` line made it pass.

```mesh
test("nested checks") do
  for status in 100..102 do
    let eligible = true
    for left in [0, 1] do
      assert_eq(left, left)
    end
  end
end
```

**Independent priority fix before promoting the intent recipe.** The developer-visible outcome is an ordinary pass or an assertion failure from nested checks. Trace the type of the outer comprehension body through MIR lowering and [list-element conversion](compiler/mesh-codegen/src/codegen/expr.rs). At HEAD the panic was at line 4456 converting a `forin_result` pointer to an integer in the Bool branch. The line identifies the failed assumption, not the cause of the type disagreement. No dependency on guide distribution. Run `meshc test tests/nested.test.mpl`; today it exits 101. Accept the fix when it runs the test, exits 0, and changing the inner assertion to `assert_eq(left, 0)` fails on the second iteration with test exit 1. Run the full budget sweep with the original nested loops and the mutant policy too. A panic or an empty test run must never qualify as catching a policy bug.

**Verification scope.** I replayed the Markdown fixtures against the published binary and the workspace build at `7c162056`. Both produced the recorded E0003 text and JSON, the expected repairs and mutation failures, and the nested-test panic. The locked workspace build and core parser, type checker, code generator, formatter and lint tests passed. The staged public-archive install proof failed as recorded above. Hosted release publication and full service or cluster qualification were outside this assessment.

## Repeat the trials on a candidate build

1. Record OS, architecture, compiler source SHA or release archive digest, compiler version, host version, model, effort and trust choices. On macOS arm64, fetch the baseline artifacts with the commands below in an empty directory. For the candidate use its own compiler and runtime libraries. Use an isolated home for installer trials.
2. Run each task in its own `meshc init` project with a fresh session. Supply an absolute compiler path if it is outside PATH. Start without a Mesh skill. Repeat with the same model, prompt and project state after running `meshc skills setup` with project scope. Keep the expected table outside the writable project. Record the compiler's exit status, tool calls, skill file reads, developer corrections, and actual executable output. Use a fresh output path or gate execution with `&&`.
3. For task A give the agent only this request: “Write a small Mesh program. Use a slot pipe to place \"middle\" in the second argument of a three-string formatter. It must print left:middle:right. Build and run it. Report which local Mesh guidance you used.” Compare with `printf 'left:middle:right\n'`. Use the failing source above separately for a deterministic diagnostic regression.
4. For task B, seed `main.mpl` with the source below. Ask it to diagnose the build, retry only timeouts, preserve all three printed rows and retain compile-time coverage of future variants. Run `meshc build . --no-color`, then `meshc build . --json` before the repair. Expect E0012 and a nonzero exit. After repair expect `printf 'false\nfalse\ntrue\n'`. Add `Pending` after `TimedOut` **in the type declaration only**. Expect E0012 naming Pending.
5. For task C use the second source below and build it before asking for the change. Request 429 under the existing strictly positive budget rule, with signature and calls preserved. Compare both binaries' stdout with the five rows in the walkthrough. As extra diagnostic controls, change the first `retry(503, 1)` to `retry("503", 1)` and, separately, `retri(503, 1)`. Restore the clean source between mistakes. Re-run the named test on the correct and both wrong implementations.

### Fetch the released compiler

```sh
gh release download v0.1.8 --repo snowdamiz/mesh-lang \
  -p meshc-v0.1.8-aarch64-apple-darwin.tar.gz -p SHA256SUMS
grep 'meshc-v0.1.8-aarch64-apple-darwin.tar.gz$' SHA256SUMS | shasum -a 256 -c -
tar xzf meshc-v0.1.8-aarch64-apple-darwin.tar.gz
mkdir -p prefix/bin prefix/lib
mv meshc prefix/bin/
mv libmesh_rt.a libmesh_test_rt.a prefix/lib/
export PATH="$PWD/prefix/bin:$PATH"
meshc --version
```

### Seed the incomplete match

```mesh
type Delivery do
  Delivered
  Rejected
  TimedOut
end

fn retry(state :: Delivery) -> Bool do
  case state do
    Delivered -> false
    Rejected -> false
  end
end

fn main() do
  println("#{retry(Delivered)}")
  println("#{retry(Rejected)}")
  println("#{retry(TimedOut)}")
end
```

### Seed the original retry policy

```mesh
fn retry(status :: Int, left :: Int) -> Bool do
  status == 503 && left > 0
end

fn main() do
  println("#{retry(503, 1)}")
  println("#{retry(503, 0)}")
  println("#{retry(429, 1)}")
  println("#{retry(429, 0)}")
  println("#{retry(500, 1)}")
end
```

### Run the independent budget check

After the change, move the function to `policy.mpl` as `pub fn retry` and import it in `main.mpl` with `from Policy import retry`. Save this file as `tests/budget.test.mpl`. Run `meshc test tests/budget.test.mpl`. Require the named test and one passing file. The test runner permits an empty discovery to exit successfully.

```mesh
from Policy import retry

test("retry eligibility across HTTP statuses and remaining budgets") do
  for status in 100..600 do
    let eligible = case status do
      429 -> true
      503 -> true
      _ -> false
    end
    assert_eq(retry(status, -1), false)
    assert_eq(retry(status, 0), false)
    assert_eq(retry(status, 1), eligible)
    assert_eq(retry(status, 2), eligible)
    assert_eq(retry(status, 8), eligible)
    0
  end
end
```

The explicit final `0` is the value of each iteration. The test's assertions run on the imported production policy. With the precedence mutant the sweep must fail on negative budget. The five-row executable check separately fails on zero. With the budget-equals-one mutant it must fail on budget two. Running the positive, negative and restored cases keeps a compiler failure or stale executable from being mistaken for evidence of correctness.
