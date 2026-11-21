# Verify laws and proofs

Use `meshc verify` to check pure code. Write the requirements and proofs in
Mesh. Keep runtime tests for behavior outside the proof model and to replay
reported counterexamples on the executable.

## Choose a verification form

| Need | Form | Checker |
| --- | --- | --- |
| Compare a pure scalar function with an independently stated expected result | `.law` file with `law name(...) -> ResultType do ... end` | Z3 4.15.7 for supported `Int` and `Bool` arguments and results |
| Prove a result law about a recursive function | Result law with a same-file `proof name do ... end` | Lean 4.36.0 |
| State a property such as idempotence or sortedness, or reuse a lemma | `.law` file starting with `logic`, with paired `theorem` and `proof` declarations | Lean 4.36.0 |
| Require proof evidence in ordinary code | `logic do ... end` in a `.mpl` file and `Proof<...>` annotations | Lean 4.36.0 during build and test |

Set `MESH_VERIFY_Z3` or `MESH_VERIFY_LEAN` to the trusted executable, or
put the pinned checker on `PATH`. For `--verdict`, install the full Lean
toolchain with `leanexport` and `con-leche` alongside `lean`.

## State a scalar policy independently

Save this function in `policy.mpl`.

```mesh
pub fn retry(status :: Int, left :: Int) -> Bool do
  let eligible = case status do
    429 -> true
    503 -> true
    _ -> false
  end
  eligible && left > 0
end
```

Save the expected decision in `laws/retry.law`.

```text
from Policy import retry

law retry(status :: Int, left :: Int) -> Bool do
  case status do
    429 -> left > 0
    503 -> left > 0
    _ -> false
  end
end
```

Run `meshc verify . --list --json` to confirm the intended declaration is
bound. Then run `meshc verify . --json` and check that the law reports
`verified`. The project can be a library without `main.mpl`. For a private
function in the same package, a result law can use
`from Policy import private retry`.

To check the boundary, append `&& left <= 8` to the implementation and
rerun verification. Expect exit 1 and `falsified`. Replay `inputs` against
the compiled function and compare `expected` with `actual`. Restore the
implementation and rerun verification.

## Prove a named property

Save these theorems in `laws/retry-properties.law` beside the result law.

```text
logic
from Policy import retry

theorem accepts_429() do retry(429, 1) == true end
proof accepts_429 do refl end

theorem some_429_may_retry() do
  exists left :: Int do retry(429, left) == true end
end
proof some_429_may_retry do
  witness 1 do exact accepts_429() end
end
```

Run `meshc verify . --verdict` with both checkers available for this project.
`refl` proves equality by reduction, and `exact` applies a term with the
required type. `witness` supplies the value in an existential. Proofs can
apply theorems from other discovered `logic` files in the package.

The existential still holds at budget `1` if you cap the implementation at
`8`. The result law catches the regression outside that witness. In this
example, `case status` keeps the compiled model within Lean's permitted
axioms. Replacing it with `status == 429 || status == 503` can leave the
theorems `unverified` even when Z3 verifies the result law.

Use `induction` for structural recursion, `cases` to split a value or
hypothesis, and `rewrite` with a proved equality. `intro` binds a quantified
variable or hypothesis. `split`, `left`, and `right` build conjunctions or
disjunctions. `branch` splits a decidable proposition into `yes(h)` and
`no(h)` cases. `unfold` expands a definition. `change` replaces the goal with
one equal by computation. Write `goal` to inspect an open obligation. It
leaves the theorem `unverified`.

Import the actual compiled declaration in the law. Use typed imports such
as `from Trees import mirror(Tree<Int>) -> Tree<Int> as mirror_int` to select
a specialization. Import an unconstrained generic without a signature to
check its supported parametric body. The proof passes the type and Nat
indices explicitly, as in `copy(T, n, values)`. Use `Type.Variant(...)` for
sum constructors and `Type.mk(...)` for struct construction in logic.

For checked examples of typed data, induction, dependent pairs, and user
inductives, follow the [proof tutorial](https://github.com/snowdamiz/mesh-lang/blob/main/website/docs/docs/tooling/index.md#state-a-theorem-and-reuse-its-proof).
The [recursive result-law example](https://github.com/snowdamiz/mesh-lang/blob/main/website/docs/docs/tooling/index.md#prove-a-recursive-list-function)
shows legacy `law` and `proof` syntax for structural list recursion. The
[compiled sort example](https://github.com/snowdamiz/mesh-lang/blob/main/compiler/meshc/tests/fixtures/proofs/sort/requirements.law)
proves signed order and preservation of element multiplicity.

## Use checked proof evidence in ordinary Mesh

Save this as `main.mpl` in a separate project.

```mesh
logic do
  pub theorem add_zero(n :: Nat) do n + 0 == n end
  proof add_zero do
    induction n do
      Nat.zero -> refl
      Nat.succ(previous, ih) -> refl
    end
  end
end

fn accept_two(evidence :: Proof<NatEq<2, 2>>) -> Int = 2

fn main() do
  println("${accept_two(add_zero(2))}")
end
```

Run `meshc build . --output example`, then `./example`. Expect `2`.
Changing the theorem call to `add_zero(3)` fails type checking. An invalid
embedded proof fails the build before it emits a new executable.

Import a closed `pub theorem` as a value. Call a parameterized theorem
with static Nat arguments and any required proof premises. Use
`Proof<Predicate<Index>>` for supported named propositions or
`Proof<NatEq<Left, Right>>` for direct Nat equality. Indexed data can
declare a parameter such as `n :: Nat` and constructor results such as
`Vec<T, Succ<n>>`. Mesh checks these indices at compile time. The Nat index
and proof evidence carry no native payload. Expressions that compute an
evidence argument still run. See the [indexed vector and evidence tutorial](https://github.com/snowdamiz/mesh-lang/blob/main/website/docs/docs/tooling/index.md#use-proofs-in-a-mesh-program).

## Discover, preserve, and replay requirements

- Run `meshc verify . --list --json` before editing. Mesh discovers visible
  `.law` files recursively and includes embedded theorems. Set
  `[verify].law_roots` in `mesh.toml` to select sidecar directories. Hidden
  directories, build output, and nested packages are excluded by default.
- To protect a baseline in CI, use the same trusted compiler to inventory
  the trusted checkout, then verify the candidate with `--preserve` and
  that inventory. Store the inventory outside the candidate's writable
  checkout. A moved file can retain the same requirement. Dropping the law,
  rebinding its target, or changing its requirement digest fails preservation.
- Run `meshc verify . --verdict --emit-proof .proof-artifacts` to check
  required theorem exports with ConLeche. `--emit-proof` writes `.lean`
  files and, with `--verdict`, `.ndjson` exports. Replay each export with
  `con-leche --verified path/to/artifact.ndjson`. Regenerate the receipt
  after changing the implementation, law, or proof.
- Explicitly run `meshc verify` for standalone `.law` files. The build and
  test proof gate checks embedded `.mpl` proofs. A normal build can succeed
  while a standalone law is open.

`verified` means the checker accepted the property over the supported
model. `falsified` returns a concrete disagreement for automatic result
checking. `unverified` gives the rejected proof, unsupported operation,
missing checker, or exhausted budget. Use `unsupported_at` in JSON to locate
unsupported expressions. `MESH_VERIFY_TIMEOUT_MS` controls the checker's
budget. Verification succeeds when it checks at least one law and every
discovered law verifies.

Model only admitted pure, total operations. An effect or foreign call in
a transitive dependency can prevent verification. The compiled proof model
supports dependent cases with one Nat index and explicit, unguarded
constructor arms. Mesh `Int` models signed 64-bit arithmetic and proof-side
`Nat` represents natural numbers. The proof check permits Lean's `propext`
axiom. A report naming other axiom dependencies is `unverified`. A
verified proof applies to the translated checked code, with native behavior
depending on the runtime and lowering. Keep the [verification limits and CI workflow](https://github.com/snowdamiz/mesh-lang/blob/main/website/docs/docs/tooling/index.md#recheck-and-export-proofs)
at hand when deciding what a result proves.
