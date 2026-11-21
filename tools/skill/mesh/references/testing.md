# Testing behavior changes

Translate the request into expected outputs, including behavior that should
survive the change. Write a named `*.test.mpl` file in the project and keep it
with the change so the next edit runs the same regression check. Run it
before and after the edit. If the request changes a predicate, check the
boundaries on both sides.

## Add HTTP 429 to a retry policy

Suppose the current production policy is `status == 503 && left > 0`.
The request is to handle 429 under the same budget rule. Both statuses still
require a strictly positive budget. Every other status remains ineligible.

Keep the original arguments and return type. To import the policy in a test,
put the production function in `policy.mpl` and use
`from Policy import retry` in `main.mpl` and the test. If the policy is already
in a module, import it from there. For a private function in an ordinary
module, a sibling `.test-support.mpl` can expose a test-only wrapper.

Save the following test as `tests/budget.test.mpl`. The expected decision
comes from the request and the old budget rule.

```mesh
from Policy import retry

test("retry eligibility across HTTP statuses and remaining budgets") do
  for status in 100..600 do
    let eligible = case status do
      429 -> true
      503 -> true
      _ -> false
    end
    for left in [-1, 0, 1, 2, 8] do
      assert_eq(retry(status, left), eligible && left > 0)
    end
  end
end
```

Run `meshc test tests/budget.test.mpl`. Expect an assertion failure on the
original 503-only implementation. If compilation fails or the runner
discovers no tests, fix the setup before using the result.

Update the policy in `policy.mpl`.

```mesh
pub fn retry(status :: Int, left :: Int) -> Bool do
  (status == 503 || status == 429) && left > 0
end
```

Run the same test again. Check the named pass and the one passing test file.
Build and run the original program if it already exercises the policy.
For calls `(503, 1)`, `(503, 0)`, `(429, 1)`, `(429, 0)`, and
`(500, 1)`, expect `true`, `false`, `true`, `false`, and `false`
respectively.

To check that the test catches a regression, temporarily change the policy
to accept only budget 1 or remove the parentheses around the allowed
statuses. Expect a failing assertion and exit 1. Restore the policy and
rerun the test. Report the change, the named test result, and any assumptions
about the request. The sweep covers the listed budgets and HTTP statuses.

## Verify laws

Use `meshc verify . --list` to find existing laws before editing a pure
function. Keep their requirement statements intact during implementation
changes. If CI supplies a trusted inventory, pass it with `--preserve` so
deleting or weakening a requirement fails the gate.

A result law uses `law name(...) -> ResultType do ... end` to state the
expected result independently. Mesh uses pinned Z3 for supported scalar
laws. A `proof` block supplies a Mesh proof for recursive result laws.

Start a `.law` file with `logic` to state named theorems, apply proved lemmas,
and use induction, quantified hypotheses, witnesses, and dependent types.
Import the checked function from its Mesh module so the theorem refers to
the implementation that builds. Author the proof in Mesh and run
`meshc verify . --verdict` with the pinned Lean toolchain to recheck its proof
export. Inspect any `falsified` or `unverified` verdict before reporting that a
requirement holds. See the [policy and proof guide](https://meshlang.dev/docs/tooling/#verify-a-pure-policy).

An ordinary `.mpl` module can use `logic do ... end` to state and prove a
theorem. `meshc build` and `meshc test` check embedded proofs when they build
the module. Import a public closed theorem as a proof value, or apply a
public theorem to static Nat indices and its required proof arguments.
Pass the resulting `Proof<Predicate<Index>>` to a native Mesh function that
requires evidence at that index. See the [embedded proof example](https://meshlang.dev/docs/tooling/#use-proofs-in-a-mesh-program).
