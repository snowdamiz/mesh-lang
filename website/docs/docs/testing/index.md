---
title: Testing
description: Write and run tests in Mesh with meshc test — assertions, grouping, mock actors, and receive expectations
---

# Testing

Mesh includes a first-class testing framework accessible via `meshc test`. Test files use the `.test.mpl` extension and can contain individual tests, grouped tests with shared setup/teardown, mock actors, and receive assertions.

> **Autonomous clusters:** This page covers testing primitives. Continue with [Distributed Proof](/docs/distributed-proof/) for the repository-owned integration, chaos, soak, and performance gates.

## Running Tests

```bash
meshc test my-app
meshc test my-app/tests
meshc test my-app/tests/config.test.mpl
meshc test --quiet my-app
```

`meshc test` discovers all `*.test.mpl` files under the requested project root or directory target, compiles and runs each independently, and prints a summary:

```
test arithmetic is correct ... ok
test string operations/length ... ok

2 tests, 0 failures
```

On failure, the output includes the failing assertion, the expected and actual values, and the file/test name. The exit code is non-zero if any test fails.

`--quiet` prints compact progress dots instead of every test name.

The top-level `tests/` directory may also contain ordinary `.mpl` helper
modules imported by test files. `meshc test` includes those helpers, while
normal builds and published packages exclude the entire top-level `tests/`
tree so test-only APIs cannot leak into production artifacts.

Mesh source discovery does not follow symbolic links. A visible symbolic link
under the requested project root is rejected instead of being used as an alias
to test-only or out-of-project source.

## Writing Tests

Test files are standalone `.test.mpl` programs. Each `test` block defines a named test:

```mesh
test("arithmetic is correct") do
  assert(1 + 1 == 2)
  assert_eq(10, 5 + 5)
  assert_ne(3, 4)
end

test("string operations") do
  assert(String.length("hello") == 5)
  assert_eq("hello", String.to_lower("HELLO"))
end
```

## Private module test support

Place `foo.test-support.mpl` beside `foo.mpl` when a test needs a narrow bridge
to that module's private implementation. During `meshc test`, Mesh appends the
support fragment to its sibling module in the temporary test project. The
fragment can therefore call private functions and use private types without
making them public in production:

```mesh
# account.mpl
fn normalized_id(raw :: String) -> String do
  String.trim(raw)
end
```

```mesh
# account.test-support.mpl
pub fn normalized_id_for_test(raw :: String) -> String do
  normalized_id(raw)
end
```

```mesh
# tests/account.test.mpl
from Account import normalized_id_for_test

test("normalizes account IDs") do
  assert(normalized_id_for_test("  alice  ") == "alice")
end
```

The basename and directory must match exactly. A test-support file contains
ordinary Mesh declarations, not `test` blocks; helpers imported by a separate
test module must be `pub`. Normal builds, package archives, generated bindings,
and the normal LSP module graph exclude `*.test-support.mpl`, while `meshc fmt`
still formats it.

Support fragments cannot target the executable entry or root `main.mpl`, which
the runner reserves for its synthetic test harness. Move private logic that
needs direct testing into an ordinary module. Test project source paths must be
regular files and directories; `meshc test` rejects visible symbolic links in
the project tree.

## Assertions

| Assertion | Description |
|-----------|-------------|
| `assert expr` | Passes if `expr` is true; prints expression source and value on failure |
| `assert_eq a, b` | Passes if `a == b`; prints expected and actual on failure |
| `assert_ne a, b` | Passes if `a != b`; prints both values on failure |
| `assert_raises fn` | Passes if calling `fn` raises a runtime error |

```mesh
test("assertions") do
  assert(true)
  assert_eq(42, 40 + 2)
  assert_ne("hello", "world")
  assert_raises fn() do
    assert(false)
  end
end
```

## Grouping with describe

Use `describe` to group related tests. The group name appears in failure output:

```mesh
describe("string operations") do
  test("length") do
    assert(String.length("hello") == 5)
  end

  test("concat") do
    assert_eq("ab", "a" <> "b")
  end
end
```

Failed test output shows: `string operations/length ... FAIL`

## Setup and Teardown

`setup` and `teardown` blocks run before and after each test in a `describe` group:

```mesh
describe("counter") do
  setup do
    assert(true)   # runs before each test in this describe
  end

  teardown do
    assert(true)   # runs after each test in this describe
  end

  test("increments") do
    assert_eq(1, 0 + 1)
  end
end
```

`setup` and `teardown` are scoped to the `describe` block — they do not affect tests outside of it.

## In-memory secure store

Libraries that use `Host.secure_store_put`, `Host.secure_store_get`, or
`Host.secure_store_delete` can install the test runner's bounded in-memory host
adapter:

```mesh
test("persists wrapped state") do
  assert(Test.install_in_memory_secure_store())
  # Call the same public library API used in production.
end
```

The adapter uses the production host-callback framing, holds at most 256 entries
and 1 MiB, zeroizes stored values, and is cleared after each test. The builtin is
available only through `meshc test`; ordinary builds reject it and still require
platform secure-store callbacks.

## Push token fixture

Tests can provide the binary token returned by the production push callback:

```mesh
test("reads the platform push token") do
  let selector = Bytes.from_utf8("expo/raw/v1")
  let token = Bytes.from_utf8("ExponentPushToken[test]")
  assert(Test.set_push_token(selector, token))
  case Host.push_get_token(selector) do
    Ok(actual) -> assert(Bytes.secure_equals(actual, token))
    Err(_) -> assert(false)
  end
end
```

`Test.set_push_token` accepts an exact non-empty selector up to 4 KiB and any
token up to 1 MiB, including empty and non-UTF-8 values. Its callback accepts
only that selector, uses the production host framing and status codes, and is
cleared and zeroized after the test. It composes with
`Test.install_in_memory_secure_store()` in either call order. Like the
secure-store adapter, it exists only in `meshc test`; ordinary builds must
register a platform callback.

## Mock Actors

Use `Test.mock_actor` to spawn a lightweight actor owned by the current test:

```mesh
test("mock actor lifecycle") do
  let mock = Test.mock_actor(fn _message do
    "handled"
  end)
  send(mock, "hello")
end
```

`Test.mock_actor(fn(String) -> String)` returns a test-owned `Pid` you can send strings to. The helper checks for messages with a 100 ms idle timeout, ignores the callback's return value, and has no `"ok"`/`"stop"` control protocol. Mesh tracks mock PIDs and cleans them up between tests. Treat it as a lifecycle and cleanup helper; when message payload or callback behavior matters, define a normal actor in the test file and use `assert_receive` to verify observable messages.

## assert_receive

`assert_receive` waits for the current test actor to receive a message matching a pattern:

```mesh
test("receive a message") do
  let me = self()
  send(me, 42)
  assert_receive 42, 500   # pattern, timeout_ms
end
```

If the message is not received within the timeout, the test fails with the pattern and elapsed time. The default timeout is 100ms when omitted:

```mesh
assert_receive "done", 1000   # explicit timeout
```

## Coverage

Coverage requests are intentionally honest today:

```bash
meshc test --coverage my-app
```

`--coverage` currently exits non-zero with an explicit unsupported message instead of returning a stub report.

## What's Next?

- [Standard Library](/docs/stdlib/) — strings, collections, files, arithmetic, crypto, and time
- [Developer Tools](/docs/tooling/) — meshc, meshpkg, formatter, REPL, LSP
- [Concurrency](/docs/concurrency/) — actors and supervision for testing async code
