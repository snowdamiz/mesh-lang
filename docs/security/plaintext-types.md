# Plaintext source labels

`Plaintext<T>` is a copyable source label. It has the representation of `T`:
existing runtime layouts, native signatures, and unlabeled programs retain their
behavior. Wrapping an affine secret resource does not make that resource
copyable; its existing ownership rules still apply.

```mesh
fn encrypt(body :: Plaintext<Bytes>, key :: borrow AeadKey,
           nonce :: Bytes, aad :: Bytes) -> Result<Bytes, CryptoError> do
  Crypto.aead_seal(key, nonce, aad, body)
end
```

The checker keeps source labels separately from ordinary type inference. A
slice, interpolation, container, record, variant, or branch selected by labeled
data carries that dependency. Assigning an ordinary annotation does not erase
it. Function summaries carry dependencies and effects across checked module
imports. The pass checks unused functions and syntactically unreachable code as
well as code retained by the optimizer.

Checked record fields and variant payloads have separate projection summaries.
A public identifier can remain public beside a labeled message body; assigning
body-derived bytes to that identifier keeps the dependency, including through
record updates and imported forwarding functions. A `Result<Plaintext<T>, E>`
can have a public success/error tag while its successful payload remains private.
The genuine `Result.is_ok`, `Result.is_err`, `Option.is_some`, and
`Option.is_none` read only that tag. `unwrap_or` retains the dependencies of the
selected successful payload and its fallback; an unused error payload is not
read. A private error payload returned on a public error branch does not control
the normal success path. A private discriminant or branch decision still does.
Bodyless constructor arms, such as `Ok(value)` in a `case`, rebuild that matched
variant, just like `Ok(value) -> Ok(value)`. Their summaries include the selected
payloads and branch control, including nested constructors, rather than unused
payloads of other variants. This does not clear a selected private payload,
private tag, guard, prior selection, or affine ownership requirement.
Wrapping the whole result in `Plaintext`, selecting its tag using private data,
matching a private literal, or using a private case guard retains the relevant
control dependency. The same distinction applies to list length and elements.
The genuine list `length`, `head`, `get`, `append`, `concat`, `reverse`, `take`,
`drop`, `tail`, and `map` operations preserve these structural proofs. Index and
count dependencies remain private when derived from content, and a mapped
callback is checked with the element's field shape and its own effects. These
rules do not apply to an imported or user-defined module with the same name.
`for ... in ... do` results likewise retain separate list length and item
projections, including nested records and imported summaries. Their length
depends on the input count, filter and pattern choices, and conditional exits;
it does not become private merely because every produced item contains a
private body. A private filter, `break`, `continue`, early return, or `?` still
keeps the relevant selection or exit dependency.
The built-in list concatenation operators `++` and `<>` preserve the same
count and element proofs; string concatenation still carries all content
dependencies.
The genuine empty constructors `List.new()`, `list_new()`, and `[]` start with
an empty structural proof, including inferred item fields. A typed accumulator
can therefore append private bodies without making its count or independent
identifiers private. Surrounding control and explicit whole-list labels remain
dependencies.

The genuine `Bytes.write_u64_be`/`bytes_write_u64_be` operation always succeeds
with eight bytes for a typed `U64`. Its status and fixed encoded size are
therefore independent of the integer's bits. `Bytes.length` preserves that size
proof through aliases and function summaries, while the encoded bytes remain
private. Caller control and a fresh explicit whole-value label still apply to
every projection. Narrowing, text, and variable-width writers retain their
input-dependent failure status; matching user-defined names gain no proof.

Genuine `Crypto.sha256`/`crypto_sha256` and `Crypto.sha512`/`crypto_sha512`
return fixed32/64-byte digests for valid typed byte inputs. Their byte length is
independent of input contents and input length, while the digest bits retain
all private dependencies. Hash bytes therefore remain refused at HTTP, log,
serialization, native, nonce, and AAD exits. Caller control, fresh whole-value
labels, and local or imported lookalikes receive no width exception. These APIs
return bytes directly; this proof grants no error-tag exception to another API.

Genuine `Bytes.concat`/`bytes_concat` preserves both operands' private byte
contents. Its result tag, overflow error, and successful byte length depend on
operand lengths and caller control rather than the copied bits. This is not a
fail-free concatenation guarantee: private extents and private branch choices
still make those facts private. It allows fixed-width private words or digests
to be framed before encryption, without allowing the frame itself to escape.
Local and imported functions with matching names receive no exception.

Genuine `Bytes.slice`/`bytes_slice` keeps the copied bytes private. Its fallible
tag and bounds error depend on the source byte length, start, count, and caller
control; successful byte length is the requested count. Selecting that success
still observes the result tag. A private extent or range therefore cannot leak
through a status branch or error, and private slice bytes cannot become public
payload, nonce, or AAD. Local and imported lookalikes gain no bounds proof.

Unknown shapes fall back to the containing value's dependencies. Source shape
expansion is bounded to twelve levels and 512 nodes; reaching that bound loses
precision conservatively rather than clearing labels.
Nested labels inside containers without projection models, such as `Map`,
`Set`, or `Queue`, conservatively label the containing value. Their metadata
does not receive the independent-field precision supported for lists.

Local `BytesBuilder` framing retains content dependencies through moves,
borrowed writer helpers, and `finish`. Summaries export mutation and escape
effects across module imports. Branches conservatively join builder contents.
An unknown/native observer makes a builder ineligible for later labeled writes,
even if it was empty when observed. Repeated allocations at one source site
cannot reset older aliases' escape facts. Existing affine ownership still
enforces use after finish and resource lifetime. This local framing capability
does not authorize JSON serialization or an arbitrary builder implementation.

The genuine `Secret.destroy`/`secret_destroy` operation zeroizes a local opaque
handle and is not an external exit. Its value and control dependencies remain
tracked, so this exception cannot clear a label or privilege a user-defined
function with the same name.

Implicit early returns through `?` carry tag dependencies, including through
nested calls, blocks, and let initializers. A public error/status that depends on content still needs a
reviewed metadata-only disclosure. Pure framing helpers may return labeled
bytes for sealing without clearing that label.

HTTP and WebSocket calls, logging, panics, serialization, native ABI boundaries,
unknown callbacks, and messages to unproven actor destinations refuse labeled
dependencies with diagnostic `E0093`. The same applies to an exit controlled by
a labeled predicate even when its payload is a public constant. This is a
conservative source analysis, not a timing or termination noninterference proof.

## Reviewed exits

The data argument of the genuine `Crypto.aead_seal`, `Crypto.hpke_seal`, and
`StorageKey.seal_bytes` builtins accepts plaintext. Their nonce, AAD, HPKE info,
storage context, and control dependencies remain subject to the checker. A user
function or imported module with a matching spelling acquires no sealing
privilege. Checked wrappers work through their effect summaries.

Only successful encrypted bytes become public. A bounded seal can fail because
of its input length, and `CryptoError.InvalidLength` can carry that length in
its payload. The result tag and error payload retain the length dependency.
Consequently, a branch, `?`, log, serialization, or native return controlled by
an unknown private length still requires a reviewed disclosure. Known public
lengths, including fixed-width integer framing, preserve legitimate ciphertext
transport without making the input bits public.

`declassify(value, "review reason")` returns the same value with the current
label removed. It is a compiler identity operation, not a runtime export API.
The reason must be a nonempty literal after escape decoding, without control
characters. Direct and piped calls are supported; the builtin cannot
escape as a function value. An explicitly labeled parameter or annotation still
introduces its declared label.

```mesh
fn preview(body :: Plaintext<String>) do
  println(declassify(body, "user enabled notification previews"))
end
```

`declassify_discriminant(result, "review reason")` reviews only a `Result`'s
success/error tag. Its type is `Result<T, E> -> Result<T, E>` with the additional
literal reason argument. It returns the identical value and preserves affine
ownership: moving a result that contains a secret moves that secret once; a
borrow cannot become an owned result. It has no runtime symbol or data export.

```mesh
fn encrypted(body :: Plaintext<Bytes>, key :: borrow AeadKey,
             nonce :: Bytes) -> Bytes do
  let sealed = Crypto.aead_seal(key, nonce, Bytes.empty(), body)
  let reviewed = declassify_discriminant(sealed,
    "application reviewed the bounded seal admission status")
  Result.unwrap_or(reviewed, Bytes.empty())
end
```

This example deliberately releases whether sealing succeeded, including the
length-dependent admission decision. It does not release an `Ok` or `Err`
payload: private plaintext and an `InvalidLength` error's actual length remain
refused at exits. A private nonce, AAD, caller branch, earlier private selection,
or implicit early return stays private. Review must precede a match, `?`, or
selection that would propagate a private tag into later control. Reviewing a
separate Boolean does not change the original result's tag.
Prior selection control is preserved independently through checked helper
summaries, closure captures, and nested aggregates. An unknown or truncated
structural proof retains the containing dependency conservatively. Reviewing
the result returned by a helper cannot undo private branch choices that helper
already used to construct or select its payload.

The current conservative source model treats an explicit whole
`Plaintext<Result<T, E>>` label as a dependency of its shape control as well as
its payloads. Tag review does not erase that stronger source contract; use the
raw fallible operation at the intended review boundary. A fresh whole-value
annotation also restores its declared label. The operation is Result-only;
`Option`, arbitrary enums, and other values receive no exception. As with
`declassify`, direct and piped calls require a nonempty literal reason without
control characters and cannot escape as a function value. Local, shadowed, and
imported functions with the same name retain ordinary call semantics.

A stable native export may instead declare a reviewed display boundary:

```mesh
@display("chat history displayed by the native host")
@export("chat_display")
pub fn display(value :: Plaintext<Bytes>) -> Result<Bytes, String> do
  Ok(value)
end
```

The existing stable export signature remains `(Bytes) -> Result<Bytes, String>`.
`@display` permits the returned display value; it does not permit logging or
network calls inside the export. Both decorator orders are accepted. A blank
reason, a duplicate display decorator, or a display decorator without a bodyful
export is rejected.

Direct messages to a locally proven actor are checked against that actor's
state and mailbox effects. Unknown or returned/indirect actor handles remain
conservative boundaries; this first implementation does not grant locality to
an arbitrary `Pid` annotation. Some safe higher-order, remote-actor, and mutable
builder patterns may therefore need a more precise proof before they compile.
Changing a function name to resemble a seal is not a proof.

## Build report

Every successful `meshc build` writes `<artifact>.plaintext.json`, including a
report with no review sites. `--plaintext-report <path>` selects another output.
`meshc test` does not write reports for its temporary executables. The schema is
versioned and contains the compiler package version, policy name, every checked
module's source SHA-256, and each `declassify`, `declassify_discriminant`, and
`@display` site's reason,
byte span, and one-based line and character column. Modules and sites are sorted;
optimization levels do not change the source inventory. Reports are emitted only
after a successful build; callers must bind the report to that build, rather than
reuse an old report after a failed command.

The report provides an inventory for review. It does not approve the reasons,
pin a compiler source revision, audit native code, or restrict a display host's
subsequent behavior. A product claim additionally needs complete content-field
labeling, reviewed disclosure changes, pinned compiler and application sources,
native-module/network inventory, and closure of UI networking. Unlabeled legacy
code does not acquire a plaintext guarantee merely by compiling with this pass.
