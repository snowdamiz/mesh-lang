# Plaintext Types

Status: implemented in the compiler (`compiler/mesh-typeck/src/plaintext.rs`),
with tests in `compiler/mesh-typeck/tests/plaintext.rs` and
`compiler/meshc/tests/e2e_plaintext.rs`.

This contract says how a Mesh program marks message content and where the
compiler lets that content go. A messenger built on it can say: in the Mesh
code that was compiled, message content leaves only through a seal, a
reviewed display export, or a reviewed declassification.

## The claim

A value of type `Plaintext<T>` reaches no way out of the program except:

| Exit | What it is for |
|---|---|
| `Crypto.aead_seal_plaintext(key, nonce, aad, Plaintext<Bytes>)` | Encryption (the ratchet, group and attachment seals) |
| `Crypto.hpke_seal_plaintext(recipient, info, aad, Plaintext<Bytes>)` | Encryption to a public key |
| `Plaintext.seal_for_storage(Plaintext<Bytes>, key, context)` | Local storage, sealed under a `StorageKey` |
| A library export marked `@display` | The host UI shows the content, or hands it in |
| `declassify(value, "reason")` | A deliberate disclosure, with a written reason |

Every `declassify` and every `@display` export is listed in the plaintext
report, which a project commits and checks in CI.

## The type

`Plaintext<T>` is a nominal type the compiler knows. It has no trait impls:
no `Display`, `Debug`, `Eq`, `Ord`, `Hash`, `ToJson` or `FromJson`. At run
time it is laid out as `T`; the label costs nothing. It enters a program
through:

- `Plaintext.from(value)`: text the program writes itself (a system message),
  or data it decides to treat as content. Labeling never discloses anything.
- `Crypto.aead_open_plaintext`, `Crypto.hpke_open_plaintext` and
  `Plaintext.unseal_from_storage`, which label what they open.
- An `@display` export's `Plaintext<Bytes>` parameter: what the host passes in.

Because no ordinary function takes a `Plaintext`, every sink that takes
`String`, `Bytes`, `Int` or another type refuses it: `println`, `IO`,
`Host.log_redacted` and the other host callbacks, `panic`, `File`, `Env`,
`Http` URLs, headers and bodies (and so push payloads), `Ws` and `WsClient`,
the databases, `Crypto.sha256`, string concatenation. String interpolation,
`Json.encode`, `inspect`, `==`, `compare` and hashing need a trait impl it
does not have. These are type errors, noted with the exits above, or E0093.

## Values that hold plaintext

A struct or sum type with a field whose type holds a `Plaintext` holds
plaintext itself, through any number of types (`TypeRegistry::plaintext_types`,
exported across modules). Such a type:

- gets none of the default derives (`Debug`, `Eq`, `Ord`, `Hash`);
- may not derive `Json`, `Display`, `Debug`, `Row`, `Eq`, `Ord` or `Hash`
  (E0093); `Schema`, which only names fields, is allowed;
- may have impls the program writes, which cannot read the content.

A generic impl applies to a type argument holding plaintext only when that
argument has the trait itself (`TraitRegistry::elements_have_it`): a derived
`Display` of `Box<T>` does not show a `Box<Plaintext<String>>`. `Option`,
`Result`, `List`, `Map`, `Set` and tuples already work this way. A `Map` or
`Set` keyed by plaintext, and `contains` over plaintext, are refused: a lookup
compares keys, and its answer is not labeled.

## Computing on content

`Plaintext.map(value, f)` and `Plaintext.map2(a, b, f)` apply `f` to the
content and label what it returns. Slices, concatenations, trimming,
splitting, lengths and comparisons are maps, and so is building a record from
content. The function must have no exits, which the checker proves:

- It is a closure written at the call, or a named top-level function of the
  program (or an imported module's function that module proved pure).
- Its body, and every function of the program it calls (a fixed point over
  the module's functions, exported as `pure_functions`), may call only the
  standard-library modules with no exits: `String`, `Bytes`, `BytesBuilder`,
  `List`, `Map`, `Set`, `Tuple`, `Range`, `Queue`, `Iter`, `Option`,
  `Result`, `Int`, `Float`, `Math`, `U64`, `U128`, `I128`, `Checked`,
  `Base64`, `Hex`, `Json`, `Regex`, `Crypto`, `DateTime`, `Duration`,
  `Monotonic`, `Random` and `Plaintext`.
- It may not send, spawn, receive, link or use `self()`, call `println`,
  `panic` or any other bare built-in, call native (`@native`) code, or call
  a function value it cannot see (a parameter, a record's field, a closure
  bound outside it).
- It may not interpolate, compare, hash, encode, iterate or pass to a
  dispatching standard-library function a value of a type the program
  defines, or of a type variable: that type's trait impls could be code with
  exits. Only the built-in types' impls are known.

`declassify` inside a map is allowed and reported like any other.

A runtime panic inside a map (`List.get` past the end, say) still ends the
actor or program, but its message is replaced by "a computation on plaintext
failed; its message is withheld": compiled code calls `mesh_plaintext_enter`
and `mesh_plaintext_leave` around the map, and the runtime's panic paths
check the depth (`compiler/mesh-rt/src/panic.rs`).

## `declassify`

`declassify(value, "reason")` returns the `T` of a `Plaintext<T>`. It must be
called directly (not piped into or passed as a value), and its reason must be
a non-empty string literal without interpolation. No program may define a
function, parameter or binding named `declassify`, or a type or module named
`Plaintext`, or import `Plaintext`'s functions by another name.

## Exports

A library export (`@export`) may use `Plaintext<Bytes>` for its parameter or
its `Ok` value only when it is also marked `@display`, before or after the
`@export`. `@display` on an export carrying no plaintext, or on a function
that is not exported, is an error. The C ABI is unchanged: a
`Plaintext<Bytes>` crosses as bytes. `@cluster` functions, which run on other
nodes, may not take or return plaintext.

## Actors

Plaintext goes only to the program's own actors. An own actor is one the
program starts on its own node with `spawn`, or one of its services, reached
through the typed `Pid<M>` that `spawn`, `self()` or the service's `start`
gives. The checker enforces:

- A message holding plaintext is not sent to an untyped `Pid` (what
  `Process.whereis` and `Global.whereis` return).
- A `Pid<M>` whose message type holds plaintext does not unify with an
  untyped `Pid` (`InferCtx::untyped_pid_conversions`), so it cannot be made
  from a registry lookup or any other untyped pid. The one exception is the
  pid handed to `Process.register`, `Global.register` or `Process.monitor`:
  the runtime keeps it, and what a lookup gives back is untyped again, which
  takes no plaintext. A service whose calls or casts take plaintext has the
  pid type `Pid<Plaintext<()>>`, so it too is reached only through its own
  pid.
- `Node.spawn` and `Node.spawn_link` take no plaintext argument and start no
  actor whose messages hold plaintext.

A pid from another node arrives only in a message from a node running a
program, which, checked the same way, sends no plaintext-bearing pid across
nodes. A closure cannot cross nodes at all (the runtime refuses to send code),
so a closure that captured plaintext stays on its node.

## The plaintext report

`meshc build <dir> --plaintext-report <file>` writes the report after a
successful build; `meshc plaintext-report <dir>` prints it, `--output <file>`
writes it, and `--check <file>` compares it with a committed one. `meshc
test` writes no report: test builds are not released.

```json
{
  "format": "mesh-plaintext-report/1",
  "package": "messenger-mobile-core",
  "declassify": [
    {
      "file": "mobile/padding.mpl",
      "line": 12,
      "function": "bucket",
      "reason": "padding bucket"
    }
  ],
  "display": [
    {
      "file": "mobile_core.mpl",
      "line": 40,
      "function": "show_message",
      "symbol": "mesh_messenger_show_message"
    }
  ]
}
```

- `file` is relative to the project directory, `/`-separated; a path
  dependency outside it is `../` relative.
- `line` is 1-based: the call for a `declassify`, the function's name for an
  export. `function` is the enclosing function (`Type.method` in an impl, the
  actor or service name inside one).
- Both lists are sorted by file, line, function and reason or symbol, and
  the JSON is pretty-printed with a final newline, so the same source gives
  the same bytes.
- `--check` compares the sites without their lines: it fails, listing each
  site added (`+`) or removed (`-`), when a `declassify` or `@display` export
  was added, removed or changed, and passes, with a note, when sites only
  moved.

A project commits its report and runs `meshc plaintext-report . --check
plaintext-report.json` in CI; release notes publish it with the source
revision it was checked at.

## What it does not claim

- It holds for the source that was compiled. Knowing that an installed binary
  is that build needs build transparency, which this does not provide; a
  claim names the source revision it was checked at.
- Code outside Mesh is not covered: the host application, native libraries
  (`@native` bindings), the Rust runtime and third-party code. What the host
  does with an `@display` export's content is the host's.
- It does not stop a compromised device.
- A computation on plaintext can differ in how long it takes, in how much it
  allocates, and in whether it crashes; those channels remain.
- A `declassify` discloses what its reason says, and more if the reason is
  wrong: the report makes each one reviewable, not correct.
