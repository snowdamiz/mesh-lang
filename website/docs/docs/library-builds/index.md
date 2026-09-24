---
title: Library Builds
description: Build a Mesh project as a static or dynamic library, call its exported functions from C, Swift, Kotlin, or TypeScript, and serve its host callbacks.
---

# Library Builds

`meshc build --artifact staticlib` or `--artifact cdylib` compiles a project
into a library that another program links and calls. The host application, such
as a mobile app, passes request bytes to an exported Mesh function and receives
response bytes or an error message. Mesh code calls back into the host through
the `Host` module.

This is the opposite direction from [Native Packages](/docs/native-packages/),
which call C code from Mesh.

## A minimal library

A library is an ordinary project. Its entry module must exist, but it does not
need a `fn main()`:

```toml
[package]
name = "greeter"
version = "0.1.0"
```

`greeter/main.mpl` exports three functions:

```mesh
@export("greeter_greet")
pub fn greet(request :: Bytes) -> Bytes!String do
  let name = Bytes.to_utf8(request)?
  if String.length(name) == 0 do
    Err("name is empty")
  else
    Ok(Bytes.from_utf8("Hello, #{name}!"))
  end
end

@export("greeter_remember")
pub fn remember(request :: Bytes) -> Bytes!String do
  case put_request(Bytes.from_utf8("greeting"), request) do
    Ok(frame) -> Host.secure_store_put(frame)
    Err(_) -> Err("greeting is too long")
  end
end

@export("greeter_recall")
pub fn recall(request :: Bytes) -> Bytes!String do
  Host.secure_store_get(Bytes.from_utf8("greeting"))
end

fn put_request(key :: Bytes, value :: Bytes) -> Bytes!BinaryError do
  let builder = BytesBuilder.new(4 + Bytes.length(key) + Bytes.length(value))?
  BytesBuilder.write_u32_be(builder, Bytes.length(key))?
  BytesBuilder.write_bytes(builder, key)?
  BytesBuilder.write_bytes(builder, value)?
  BytesBuilder.finish(builder)
end
```

Build it from the parent directory:

```bash
meshc build greeter --artifact staticlib
```

It writes the archive and six binding files:

```text
greeter/libgreeter.a
greeter/libgreeter.h
greeter/libgreeter.swift
greeter/libgreeter.kt
greeter/libgreeter.jni.c
greeter/libgreeter.ts
greeter/libgreeter.abi.json
```

A C host includes the generated header, links the archive, and calls the
exports. This host also provides the two secure-store callbacks the library
uses:

```c
#include <stdio.h>
#include <string.h>
#include "libgreeter.h"

/* One record, for brevity. */
static uint8_t saved[256];
static uint64_t saved_len = 0;
static int has_value = 0;

/* input: 4-byte big-endian key length, key, value */
static int32_t store_put(void *context, const uint8_t *input, uint64_t input_len,
                         uint8_t *output, uint64_t output_capacity, uint64_t *output_len) {
  (void)context; (void)output; (void)output_capacity; (void)output_len;
  if (input_len < 4) return 1;
  uint64_t key_len = ((uint64_t)input[0] << 24) | ((uint64_t)input[1] << 16) |
                     ((uint64_t)input[2] << 8) | input[3];
  if (key_len > input_len - 4 || input_len - 4 - key_len > sizeof saved) return 1;
  saved_len = input_len - 4 - key_len;
  memcpy(saved, input + 4 + key_len, saved_len);
  has_value = 1;
  return 0;
}

/* input: the key. Status 2 means there is no record. */
static int32_t store_get(void *context, const uint8_t *input, uint64_t input_len,
                         uint8_t *output, uint64_t output_capacity, uint64_t *output_len) {
  (void)context; (void)input; (void)input_len;
  if (!has_value) return 2;
  if (saved_len > output_capacity) return 4;
  memcpy(output, saved, saved_len);
  *output_len = saved_len;
  return 0;
}

static void print_result(const char *label, int32_t status, MeshLibraryBytes *response) {
  printf("%s: status=%d %.*s\n", label, status, (int)response->len,
         response->data ? (const char *)response->data : "");
  mesh_library_free_returned_bytes(response);
}

int main(void) {
  if (mesh_library_init() != MESH_LIBRARY_OK) return 1;

  MeshLibraryHostCallbacksV1 callbacks = {0};
  callbacks.abi_version = MESH_LIBRARY_ABI_VERSION;
  callbacks.struct_size = sizeof callbacks;
  callbacks.secure_store_put = store_put;
  callbacks.secure_store_get = store_get;
  if (mesh_library_register_host_callbacks(&callbacks) != MESH_LIBRARY_OK) return 1;

  MeshLibraryBytes response = {0};
  print_result("greet", greeter_greet((const uint8_t *)"Ada", 3, &response), &response);
  print_result("greet", greeter_greet(NULL, 0, &response), &response);
  print_result("recall", greeter_recall(NULL, 0, &response), &response);
  print_result("remember", greeter_remember((const uint8_t *)"Hi", 2, &response), &response);
  print_result("recall", greeter_recall(NULL, 0, &response), &response);

  mesh_library_shutdown();
  return 0;
}
```

On macOS:

```bash
cc -Igreeter host.c greeter/libgreeter.a \
  -framework Security -framework CoreFoundation -o host
./host
```

```text
greet: status=0 Hello, Ada!
greet: status=9 name is empty
recall: status=9 host_callback_failed:2:2
remember: status=0
recall: status=0 Hi
```

The first `recall` fails because the store is empty: the callback returned
status `2`, which Mesh receives as an `Err`, and the exported function returned
that `Err` to the host as status `9`.

## Build options

| `--artifact` | Output | Default path |
| --- | --- | --- |
| `executable` (default) | Native program | `<dir>/<name>` |
| `staticlib` | Static archive | `<dir>/lib<name>.a` |
| `cdylib` | Dynamic library | `<dir>/lib<name>.dylib` for Apple targets, `<dir>/lib<name>.dll` for Windows, `<dir>/lib<name>.so` otherwise |

`<name>` is the last component of the directory argument, not the package name
in `mesh.toml`. `meshc build greeter --artifact staticlib` writes
`greeter/libgreeter.a`, but `meshc build . --artifact staticlib` writes
`./liboutput.a`, because `.` has no name. Use `--output` to choose the path:

```bash
meshc build . --artifact cdylib --output dist/libgreeter.dylib
```

The bindings are written beside the artifact: each takes the artifact path and
replaces its extension, so `dist/libgreeter.dylib` also produces
`dist/libgreeter.h`, `dist/libgreeter.swift`, and so on.

For a library build:

- The entry module (`main.mpl`, or `[package].entrypoint`) must exist. It does
  not need `fn main()`; if there is one, it is compiled but never run, and no C
  `main` is emitted.
- At least one function must be exported. Otherwise the build fails with
  ``library artifacts require at least one `@export` function``.
- `--emit-llvm` writes the IR beside the artifact with the extension `.ll`
  (`greeter/libgreeter.ll`), without the program entry point.
- `--opt-level` and `--target` work as for executables. See
  [Tooling](/docs/tooling/#build-projects).

A library contains the compiled project, the Mesh runtime, and the static
archives of any [native packages](/docs/native-packages/) it depends on. Every
library carries its own copy of the runtime, and the generated Swift and Kotlin
bindings use fixed type names, so give an application one Mesh library that
exports everything it needs.

When you link the static archive yourself, add the system libraries the runtime
uses. On macOS and iOS these are the `Security` and `CoreFoundation`
frameworks. On other Unix-like targets, `meshc` links its own executables
against the same runtime with only `-lm`.

## Exported functions

`@export("symbol")` makes a function callable from the host under that C
symbol. The only accepted form is:

```text
@export("c_symbol")
pub fn name(request :: Bytes) -> Bytes!String do ... end
```

The function must:

- be `pub`;
- take exactly one parameter, annotated as `Bytes`;
- declare the return type `Bytes!String` (`Result<Bytes, String>`);
- have no generic parameters, `where` clause, or guard; and
- name a C identifier: an ASCII letter or `_`, then letters, digits, or `_`,
  other than `main`, a C keyword, or a name starting with `mesh_library_`
  (the host ABI below).

Anything else is error E0055:

```text
[E0055] Error: invalid exported function declaration
   ╭─[ main.mpl:1:1 ]
   │
 1 │ ╭─▶ @export("bad_sig")
   ┆ ┆
 4 │ ├─▶ end
   │ │
   │ ╰───────── stable exports require exactly `(Bytes) -> Result<Bytes, String>`
   │
   │     Help: use `@export("c_symbol") pub fn name(request :: Bytes) -> Bytes!String`
───╯
```

Exports may be declared in any module of the project. Two exports with the same
symbol fail the build with `duplicate exported symbol '<symbol>'`. A symbol
that matches a function the compiled module already defines or declares fails
with `Exported symbol '<symbol>' conflicts with another generated or runtime
symbol`. That check sees only the runtime functions the program uses, so
give symbols a prefix unique to your library rather than a bare `mesh_` one.

`Ok(bytes)` returns the bytes to the host. `Err(message)` returns the message
text with status `9`. Use `Bytes` for structured data and choose the encoding
yourself, such as JSON text or a length-prefixed binary format built with
[`BytesBuilder`](/docs/stdlib/#building-binary-values).

## Generated files

| File | Contents |
| --- | --- |
| `<out>.h` | C declarations for the ABI and every export |
| `<out>.swift` | `MeshLibrary` enum and `MeshLibraryFailure` error for Swift |
| `<out>.kt` | `mesh.MeshLibrary` object with JNI `external` functions |
| `<out>.jni.c` | JNI glue that implements the Kotlin functions in C |
| `<out>.ts` | One `Promise`-returning function per export for an Expo native module |
| `<out>.abi.json` | Machine-readable description of the library |

The manifest for the example above:

```json
{
  "abiVersion": 1,
  "artifact": "libgreeter.a",
  "exports": [
    {
      "meshFunction": "greet",
      "request": "bytes",
      "response": "result<bytes,string>",
      "symbol": "greeter_greet"
    }
  ],
  "ownership": {
    "request": "borrowed-for-call",
    "response": "caller-owned; release with mesh_library_free_returned_bytes"
  },
  "target": "aarch64-apple-darwin"
}
```

`exports` lists every export; this excerpt shows one. `target` is the
`--target` triple, or the host triple without one.

## The C ABI

An excerpt from `libgreeter.h`, which defines ABI version 1:

```c
#define MESH_LIBRARY_ABI_VERSION 1

typedef struct MeshLibraryBytes {
  uint8_t *data;
  uint64_t len;
} MeshLibraryBytes;

int32_t mesh_library_init(void);
int32_t mesh_library_shutdown(void);
int32_t mesh_library_register_host_callbacks(const MeshLibraryHostCallbacksV1 *callbacks);
void mesh_library_free_returned_bytes(MeshLibraryBytes *bytes);
int32_t greeter_greet(const uint8_t *request, uint64_t request_len, MeshLibraryBytes *response);
```

### Lifecycle

`mesh_library_init()` starts the runtime and returns `0`. Actors that exported
functions spawn run on the runtime's scheduler, which starts one worker thread
by default. Calling `mesh_library_init()` again while the runtime runs returns
`0` and does nothing.

`mesh_library_register_host_callbacks(table)` installs the host callbacks. It
must be called after `mesh_library_init`; see
[Host callbacks](#host-callbacks).

`mesh_library_shutdown()` waits for an exported call in progress to return,
stops the scheduler, removes the registered callbacks, and returns `0`.
Stopping the scheduler waits for running actors to finish; actors waiting in
`receive` are stopped. It returns `0` without doing anything when the runtime
is not running, including before `mesh_library_init`.

The runtime cannot be restarted in the same process. After shutdown,
`mesh_library_init` and every exported function return `2`.

The order is:

1. `mesh_library_init()`.
2. `mesh_library_register_host_callbacks()`, if the library uses `Host` or
   `StorageKey.platform()`.
3. Exported calls.
4. `mesh_library_shutdown()`.

### Calling an exported function

```c
int32_t symbol(const uint8_t *request, uint64_t request_len, MeshLibraryBytes *response);
```

- `request` is borrowed for the call. The runtime copies it before the Mesh
  function runs. It may be `NULL` only when `request_len` is `0`.
- `response` must not be `NULL`. When the runtime accepts the arguments, it sets
  `*response` to `{NULL, 0}` before anything else. When it rejects them with
  status `1`, it leaves `*response` untouched, so initialize it to `{0}`.
- The Mesh function runs on the calling thread, as a new Mesh process that ends
  when the call returns. Values it creates are released then.
- For `Ok(bytes)`, the call returns `0` and `*response` holds a copy of the
  bytes. For `Err(message)`, it returns `9` and `*response` holds the message's
  UTF-8 bytes. An empty result is `{NULL, 0}`.
- Requests and results, including error messages, are limited to 1 MiB
  (1,048,576 bytes). A larger request or result returns `6` with an empty
  response.
- A panic returns `4` with an empty response. The runtime prints the panic
  message to standard error and stays usable.

The caller owns the response. Release it with
`mesh_library_free_returned_bytes`, not `free`, after every call, including
failed ones. The function frees the data, resets the struct to `{NULL, 0}`, and
accepts `NULL` or an already released struct.

State that must outlive a call belongs in storage or in an actor the call
spawns. A registered [service](/docs/concurrency/#services-genserver) keeps its
state across calls until `mesh_library_shutdown`:

```mesh
service Counter do
  fn init(start_val :: Int) -> Int do
    start_val
  end

  call Increment(amount :: Int) :: Int do |count|
    (count + amount, count + amount)
  end
end

@export("counter_start")
pub fn start(request :: Bytes) -> Bytes!String do
  let pid = Counter.start(0)
  if Process.register("counter", pid) == 0 do
    Ok(Bytes.empty())
  else
    Err("already started")
  end
end

@export("counter_bump")
pub fn bump(request :: Bytes) -> Bytes!String do
  let pid = Process.whereis("counter")
  let count = Counter.increment(pid, 1)
  Ok(Bytes.from_utf8("#{count}"))
end
```

Calling `counter_start` once and `counter_bump` three times returns `1`, `2`,
and `3`. A second `counter_start` returns status `9` with `already started`.

### Status codes

| Code | Name | Meaning |
| --- | --- | --- |
| `0` | `MESH_LIBRARY_OK` | Success |
| `1` | `MESH_LIBRARY_ERR_INVALID_ARGUMENT` | A `NULL` response pointer, a `NULL` request with a non-zero length, or a `NULL` callback table |
| `2` | `MESH_LIBRARY_ERR_NOT_INITIALIZED` | The runtime is not running: before `mesh_library_init`, or after `mesh_library_shutdown` |
| `3` | `MESH_LIBRARY_ERR_BUSY` | Another exported call is in progress in this process |
| `4` | `MESH_LIBRARY_ERR_PANIC` | The Mesh function panicked |
| `5` | `MESH_LIBRARY_ERR_HOST_CALLBACK` | Reserved in ABI 1; no function returns it yet |
| `6` | `MESH_LIBRARY_ERR_OUTPUT_TOO_LARGE` | The request or result is larger than 1 MiB, or the response could not be allocated |
| `7` | `MESH_LIBRARY_ERR_ABI` | The callback table's `abi_version` is not `1` or its `struct_size` is wrong |
| `8` | `MESH_LIBRARY_ERR_CALLBACK_MISSING` | Reserved for the host side; the runtime uses it internally when a capability has no callback, and Mesh code sees that as `Err("host_callback_missing")` |
| `9` | `MESH_LIBRARY_ERR_APPLICATION` | The Mesh function returned `Err`; the response holds its message |

A failing or missing host callback does not produce status `5` or `8`. Mesh
code receives it as an `Err`, and the host sees status `9` only if the exported
function returns that `Err`.

### Threads

- Only one exported call runs at a time in a process. A call made while another
  is in progress, from any thread, returns `3` at once; it does not wait.
  Serialize calls in the host, for example on one queue.
- An exported function called from a host callback returns `3`, because the
  outer call is still in progress.
- Never call `mesh_library_shutdown` from a host callback. It waits for the
  call that is running the callback, so it never returns.
- `mesh_library_init`, `mesh_library_register_host_callbacks`, and
  `mesh_library_shutdown` may be called from any thread.

## Host callbacks

The `Host` module lets Mesh code ask the host for platform services. Each
function takes request `Bytes`, returns `Result<Bytes, String>`, and calls one
entry of the host's callback table:

| Number | Mesh function | Callback field |
| --- | --- | --- |
| 1 | `Host.secure_store_put(request)` | `secure_store_put` |
| 2 | `Host.secure_store_get(request)` | `secure_store_get` |
| 3 | `Host.secure_store_delete(request)` | `secure_store_delete` |
| 4 | `Host.push_get_token(request)` | `push_get_token` |
| 5 | `Host.background_schedule(request)` | `background_schedule` |
| 6 | `Host.network_state(request)` | `network_state` |
| 7 | `Host.monotonic_clock(request)` | `monotonic_clock` |
| 8 | `Host.wall_clock(request)` | `wall_clock` |
| 9 | `Host.log_redacted(request)` | `log_redacted` |

### The callback table

```c
typedef int32_t (*MeshLibraryHostCallback)(void *context, const uint8_t *input, uint64_t input_len, uint8_t *output, uint64_t output_capacity, uint64_t *output_len);

typedef struct MeshLibraryHostCallbacksV1 {
  uint32_t abi_version;
  uint32_t struct_size;
  void *context;
  MeshLibraryHostCallback secure_store_put;
  MeshLibraryHostCallback secure_store_get;
  MeshLibraryHostCallback secure_store_delete;
  MeshLibraryHostCallback push_get_token;
  MeshLibraryHostCallback background_schedule;
  MeshLibraryHostCallback network_state;
  MeshLibraryHostCallback monotonic_clock;
  MeshLibraryHostCallback wall_clock;
  MeshLibraryHostCallback log_redacted;
} MeshLibraryHostCallbacksV1;
```

Set `abi_version` to `MESH_LIBRARY_ABI_VERSION` and `struct_size` to
`sizeof(MeshLibraryHostCallbacksV1)`. `context` is passed unchanged as the first
argument of every callback. Leave a field `NULL` for a capability the host does
not provide.

`mesh_library_register_host_callbacks` returns `1` for a `NULL` table, `7` for
a wrong version or size, and `2` when the runtime is not running. On success it
copies the table, replacing any earlier registration, and returns `0`. The table
itself may then be released, but `context` and the functions must stay valid
until the table is replaced or `mesh_library_shutdown` returns.

### Writing a callback

- `input` holds `input_len` request bytes and is borrowed for the call.
- `output` has room for `output_capacity` bytes. Write the response there and
  store its length in `*output_len`, which starts at `0`. For `Host` calls the
  capacity is 1 MiB; the runtime's own secure-store calls pass smaller buffers,
  down to zero bytes.
- Return `0` for success. Any other value is a failure; the runtime does not
  read `output` then.
- Return normally. Do not throw, unwind, or `longjmp` out of a callback.

A callback runs on the thread that runs the calling Mesh code: the host thread
that made the exported call, or the runtime's worker thread for an actor that
call spawned. Callbacks can therefore run between exported calls and on more
than one thread at once, so make them thread-safe.

### What Mesh code receives

| Result | When |
| --- | --- |
| `Ok(bytes)` | The callback returned `0`; `bytes` are the first `*output_len` bytes of `output` |
| `Err("host_callback_not_registered")` | No table is registered |
| `Err("host_callback_missing")` | A table is registered, but this field is `NULL` |
| `Err("host_callback_failed:<number>:<status>")` | The callback returned a non-zero status, such as `host_callback_failed:2:2` |
| `Err("host_callback_input_too_large")` | The request is larger than 1 MiB |
| `Err("host_callback_output_too_large")` | `*output_len` is larger than `output_capacity` |

The runtime copies the request and response without interpreting them, so the
formats are an agreement between your Mesh code and your host. The output
buffer is zeroized after its bytes are copied.

### Secure-store format

The runtime uses the secure-store callbacks itself for
[`StorageKey.platform()`](/docs/stdlib/#sealing-secrets-for-storage), with this
format:

| Callback | Input | Output |
| --- | --- | --- |
| `secure_store_put` | 4-byte big-endian key length, the key, then the value | None |
| `secure_store_get` | The key | The value, or status `2` when the key has no value |
| `secure_store_delete` | The key | None |

`StorageKey.platform()` stores its record under the key `mesh/storage-key/v2`
and deletes the legacy `mesh/storage-key/v1` and `mesh/storage-counter/v1`
records after moving them into it. It treats status `2` from `get` or `delete` as
"not found" and any other failure as `Err(InternalFailure)`. Use the same
format when Mesh code calls `Host.secure_store_*` directly, so that both share
one store.

Under `meshc test`, `Test.install_in_memory_secure_store()` and
`Test.set_push_token(selector, token)` register callbacks that follow this
format; see [Testing](/docs/testing/#in-memory-secure-store). Those fixtures are
not part of a library build. The `Host` functions themselves are described in
[Standard Library](/docs/stdlib/#host-capabilities).

## Swift

Add `lib<name>.h` to the target's bridging header, add `lib<name>.swift` to the
target, and link the library. Link a static archive with the `Security` and
`CoreFoundation` frameworks.

```swift
import Foundation

try MeshLibrary.initialize()
defer { MeshLibrary.shutdown() }

let reply = try MeshLibrary.greet(Data("Ada".utf8))
print(String(decoding: reply, as: UTF8.self))

do {
  _ = try MeshLibrary.greet(Data())
} catch let failure as MeshLibraryFailure {
  print(failure.status, String(decoding: failure.payload, as: UTF8.self))
  print(failure.localizedDescription)
}
```

```text
Hello, Ada!
9 name is empty
Mesh library call failed (status=9): name is empty
```

`MeshLibrary` has one static method per export, named after the Mesh function
and taking and returning `Data`. It throws `MeshLibraryFailure` for any non-zero
status. `payload` holds the response bytes, which for status `9` is the error
message; the description includes the payload when it is non-empty UTF-8. Each
method releases the response itself. `initialize()` throws for a non-zero
`mesh_library_init` status, and `shutdown()` ignores the result.

Register callbacks with closures that capture nothing, since the table holds C
function pointers:

```swift
var callbacks = MeshLibraryHostCallbacksV1()
callbacks.abi_version = UInt32(MESH_LIBRARY_ABI_VERSION)
callbacks.struct_size = UInt32(MemoryLayout<MeshLibraryHostCallbacksV1>.size)
callbacks.secure_store_get = { _, input, inputLen, output, capacity, outputLen in
  let value = Array("stored greeting".utf8)
  guard UInt64(value.count) <= capacity, let output, let outputLen else { return 4 }
  output.update(from: value, count: value.count)
  outputLen.pointee = UInt64(value.count)
  return 0
}
precondition(mesh_library_register_host_callbacks(&callbacks) == MESH_LIBRARY_OK)
```

## Kotlin and JNI

`lib<name>.kt` declares `object MeshLibrary` in package `mesh`:

```kotlin
package mesh

object MeshLibrary {
    init {
        System.loadLibrary("greeter")
        val status = initializeNative()
        check(status == 0) { "Mesh library initialization failed (status=$status)" }
    }

    @JvmStatic fun ensureInitialized() = Unit
    @JvmStatic private external fun initializeNative(): Int
    @JvmStatic external fun shutdownNative(): Int
    @JvmStatic external fun greet(request: ByteArray): ByteArray
    @JvmStatic external fun remember(request: ByteArray): ByteArray
    @JvmStatic external fun recall(request: ByteArray): ByteArray
}
```

The first use of `MeshLibrary` loads the native library and calls
`mesh_library_init`; `ensureInitialized()` triggers that early. Each export
returns the response bytes or throws `IllegalStateException` with the message
`Mesh library call failed (status=<n>): <payload>`.

```kotlin
val reply = MeshLibrary.greet("Ada".toByteArray())
```

`System.loadLibrary` receives the artifact's file name without `lib` and its
extension. The library it loads must contain the functions from
`lib<name>.jni.c`, which call the exports and release their responses. `meshc`
does not compile that file; build it with the target's NDK compiler into the
`lib<name>.so` the app packages, linking the static archive. The Kotlin object
does not register host callbacks; register them from native code after
`mesh_library_init`.

## TypeScript

`lib<name>.ts` exports one function per export:

```ts
export const greet = (request: Uint8Array): Promise<Uint8Array> => native.invoke('greeter_greet', request);
```

`native` is `requireNativeModule('MeshMessenger')` from `expo-modules-core`.
`meshc` does not generate that module: the app must provide an Expo native
module named `MeshMessenger` with
`invoke(symbol: string, request: Uint8Array): Promise<Uint8Array>` that calls
the named export, for example through the Swift or Kotlin bindings above.

## Targets

`meshc` links the library with the target's own tools:

| Target | `staticlib` | `cdylib` |
| --- | --- | --- |
| macOS (`*-apple-darwin`) | `xcrun libtool -static` | `cc`, or `xcrun --sdk macosx clang -target <triple>` with `--target`; adds `Security` and `CoreFoundation` |
| iOS (`*-apple-ios`, `*-apple-ios-sim`) | `xcrun libtool -static` | `xcrun --sdk iphoneos clang -target <triple>`, or `--sdk iphonesimulator` for a triple ending in `-sim`; adds `Security` and `CoreFoundation` |
| Android (`*-linux-android`) | NDK `llvm-ar` | NDK `<triple>26-clang` |
| Linux and other Unix-like targets | `ar` | `cc -shared` (`cc -target <triple>` with `--target`) |
| Windows MSVC (`*-windows-msvc`) | Not supported | `clang.exe` from `LLVM_SYS_211_PREFIX\bin`, or `clang` on `PATH` |

The static archive contains the project object, the runtime archive, and any
native-package archives. A dynamic library includes the whole runtime.

Android builds read the NDK from `ANDROID_NDK_HOME`, or `ANDROID_NDK_ROOT`, and
look for the tools in its `toolchains/llvm/prebuilt/*/bin` directories. The
dynamic linker is named after the triple and API level 26, such as
`aarch64-linux-android26-clang`, so the triple must match an NDK compiler name.

On Windows MSVC, `--artifact staticlib` fails with `static library artifacts
are not yet supported for Windows MSVC`. A `cdylib` build writes
`lib<name>.dll`, exports every `@export` symbol and the four `mesh_library_*`
functions, and leaves an import library beside it. Link the host against that
`lib<name>.lib`.

A dynamic library records its file name alone as the name hosts load it by:
the install name `@rpath/libgreeter.dylib` on macOS, the `soname`
`libgreeter.so` on Linux. Give the host an rpath to the library's directory:

```bash
cc -Igreeter host.c -Lgreeter -lgreeter -Wl,-rpath,@executable_path/greeter -o host
```

A cross-target build also needs the Mesh runtime compiled for that target; see
[Tooling](/docs/tooling/#build-projects) for where `meshc` looks for it.

The `e2e_library` test in `compiler/meshc/tests` builds a dynamic and a static
library from a fixture, runs a C host against the dynamic library (and a Swift
host on macOS), and checks that neither contains the `meshc test` fixtures. On
Windows it builds a DLL and runs a C host against it; the cross-platform
compatibility workflow runs that Windows test.

## See also

- [Native Packages](/docs/native-packages/) for calling C from Mesh.
- [Standard Library](/docs/stdlib/) for `Bytes`, `BytesBuilder`, `StorageKey`,
  and `Host`.
- [Tooling](/docs/tooling/) for `meshc build` options and cross-target builds.
- [Testing](/docs/testing/) for the in-memory secure store and push-token
  fixtures.
