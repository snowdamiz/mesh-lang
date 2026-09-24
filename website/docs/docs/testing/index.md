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

The path is optional and defaults to the current directory. It may be a
project, a directory inside one, or one `*.test.mpl` file; the project is the
nearest enclosing directory with a `mesh.toml`, and without one the run fails
with "Could not resolve a Mesh project root". `meshc test` finds every
`*.test.mpl` file under the path, skipping hidden directories and `target/`,
and runs the files in path order. Each file is compiled, at debug optimization
and together with the project's modules, into a program of its own, and run as
its own process; the tests of one file run one after another in that process.
It prints each file's results when the file finishes, then a summary across
files:

```
  ✓ arithmetic is correct
  ✓ string operations

2 passed in 0.00s

1 test file passed in 1.76s
```

Output a test prints itself comes before its `✓` or `✗` line. A failed test is
listed again under `Failures:` after the file's tests, with what failed. The
last line counts test files, and its time includes compiling them. A file that
does not compile is reported as `COMPILE ERROR: <file>` with the compiler's
diagnostics, which point at the lines of the test file as written.

The exit status is 1 when any file fails, including one that does not
compile, and 0 otherwise, including when there are no test files (`No
*.test.mpl files found.`). Output is colored only on a terminal with `NO_COLOR`
unset.

`--quiet` prints a `.` for each passing test and an `F` for each failing one,
then the `Failures:` list and the counts.

The top-level `tests/` directory may also contain ordinary `.mpl` helper
modules imported by test files. `meshc test` includes those helpers, while
normal builds and published packages exclude the entire top-level `tests/`
tree so test-only APIs cannot leak into production artifacts.

Mesh source discovery does not follow symbolic links. A visible symbolic link
under the requested project root is rejected instead of being used as an alias
to test-only or out-of-project source.

## Writing Tests

A test file holds `test` blocks, `describe` groups, and ordinary definitions
such as helper functions. It can import the project's modules, except its
entrypoint (`main.mpl`, or the manifest's `entrypoint`), which the runner
replaces with its own. Helper modules under `tests/` are imported by their
path, as `from Tests.Support import label` for `tests/support.mpl`. Each `test`
block defines a named test; its label must be a string literal:

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
| Assertion | Passes when | Failure message |
|-----------|-------------|-----------------|
| `assert(expr)` | `expr`, a `Bool`, is `true` | `assert failed: <expr as written>` |
| `assert_eq(a, b)` | `a` and `b` show the same text | `assert_eq failed: <a> == <b>`, then `left:` and `right:` values |
| `assert_ne(a, b)` | `a` and `b` show different text | `assert_ne failed: <a> != <b>`, then `both sides equal:` |
| `assert_raises(fn)` | calling `fn` panics or fails an assertion | `assert_raises failed: expression did not raise` |

`assert_eq` and `assert_ne` take two values of the same type and compare them
as `"#{value}"` shows them, through `Display`, so both sides must implement it.

A failed assertion ends its test and fails it, as a runtime error (a panic,
such as `List.get` past the end or a division by zero) does; the run goes on
with the next test. A panic's message is the failure: `panicked: <message>`,
so `panic("...")` fails a test with a message of your own. Inside
`assert_raises`, a failed assertion is the raise it expects.

A failure that ends the whole process, such as a stack overflow, ends the file
without its summary, and the file counts as failed. There is no per-test
timeout: a test that never finishes stops the run. The tests of a file share
one process, so actors they spawn and names they register carry over to the
next test; only mock actors and the host fixtures below are reset between
tests.

```mesh
test("assertions") do
  assert(true)
  assert_eq(42, 40 + 2)
  assert_ne("hello", "world")
  assert_raises(fn() do
    assert(false)
  end)
end
```

## Grouping with describe

Use `describe` to group related tests. The group name, a string literal, is
joined to each test's name with ` > `:

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

A failed test is marked `✗` with its group and name, followed by what failed.
Had `length` asserted `assert_eq(String.length("hello"), 4)`, the output would
read:

```
  ✗ string operations > length
    assert_eq failed: String.length("hello") == 4
      left:  5
      right: 4
```

## Setup and Teardown

`setup` and `teardown` blocks run before and after each test in a `describe`
group. Both may be written with or without the empty parentheses (`setup do`):

```mesh
describe("counter") do
  setup() do
    assert(true)   # runs before each test in this describe
  end

  teardown() do
    assert(true)   # runs after each test in this describe
  end

  test("increments") do
    assert_eq(1, 0 + 1)
  end
end
```

`setup` and `teardown` are scoped to the `describe` block — they do not affect tests outside of it. A `describe` may have several `setup` blocks, which run in order, and at most one `teardown`; every `setup` must come before the first test or nested `describe` of its `describe`, or the file does not compile. Values a `setup` binds are visible in each test and in `teardown`. A failing `setup` fails the test without running its body or `teardown`; after the body, `teardown` runs whether the test passed or not.

### Nested describe

A `describe` can contain another. Its tests are named with both labels, as
`outer > inner > test`; each one runs the outer `setup` blocks and then the
inner ones, so it sees the bindings of both, and afterwards the inner
`teardown` and then the outer one.

```mesh
describe("accounts") do
  setup do
    let owner = "ada"
  end

  describe("with a balance") do
    setup do
      let balance = 10
    end

    test("belongs to its owner") do
      assert_eq(owner, "ada")
      assert_eq(balance, 10)
    end
  end
end
```

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

The adapter uses the production host-callback framing, holds at most 256
entries with keys of 1 to 4096 bytes and 1 MiB of keys and values together,
zeroizes stored values, and is cleared after each test and by each call that
installs it. The builtin is
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
token up to 1 MiB, including empty and non-UTF-8 values. A test may set up to
256 selectors, 1 MiB of tokens in all; setting a selector again replaces its
token. The callback answers only the selectors that were set (any other is
invalid input), uses the production host framing and status codes, and is
cleared and zeroized after the test. It composes with
`Test.install_in_memory_secure_store()` in either call order. Like the
secure-store adapter, it exists only in `meshc test`; ordinary builds must
register a platform callback.

## Mock Actors

Use `Test.mock_actor` to spawn a lightweight actor owned by the current test:

```mesh
test("mock actor receives messages") do
  let me = self()
  let mock = Test.mock_actor(fn message do
    send(me, "got " <> message)
    "handled"
  end)
  send(mock, "hello")
  assert_receive "got hello", 500
end
```

`Test.mock_actor(fn(String) -> String)` returns a `Pid` you can send strings
to. The actor calls the function with each message, in order, and ignores what
it returns; there is no `"ok"`/`"stop"` control protocol. It keeps waiting for
messages until the test ends: Mesh tracks mock PIDs and stops them before the
next test. The function may be a closure or a named function. To check what a
mock saw, have it send to the test, as above, and use `assert_receive`.

## assert_receive

`assert_receive PATTERN, TIMEOUT_MS` waits up to the timeout for the test's
next message and checks it against the pattern:

```mesh
test("receive a message") do
  let me = self()
  send(me, 42)
  assert_receive 42, 500
end
```

A next message that does not match fails the test at once with
`assert_receive 42 received another message`; no message in time fails it with
`assert_receive 42 timed out after 500ms`. Without a timeout, it waits 100ms:

```mesh
assert_receive "done"
```

`assert_receive` is a statement of its own, written in a `test`, `setup`, or
`teardown` block; anywhere else, even in a helper function in the test file, it
is error E0077. Names its pattern binds are not available after it.

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
