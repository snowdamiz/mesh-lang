# Direct allocation for String.join

`String.join` resolves the visible list slots, checks the final byte count,
and writes into one new Mesh string. The actor allocator can request
collection for the next compiled safepoint. The immutable inputs stay live
through both walks of the list. Empty and singleton joins still allocate a
fresh result. The join copies raw bytes, including embedded NUL, without
forming Rust UTF-8 references. [Runtime and tests](../../compiler/mesh-rt/src/string.rs)

The size check includes the String header and actor page overhead. It also
checks the actor heap's 32-bit allocation-size field. Oversized results
fail before allocation through the existing C entry point. The focused
tests cover persistent list aliases, allocation under
GC pressure, fresh results, and leaf tracing with raw native input. The
local 32-bit Wasm check compiled the production join and length calculation
with a minimal list and allocator. It exercised the size limit and byte copy
with 32-bit pointers. The production actor heap ran on macOS ARM64.

## Local results

Each CPU interval includes compiled Mesh calls and a rolling checksum.
The two Jobs profiles also include the two enclosing bracket concatenations.
The release runs use one compiler, identical LLVM IR per pair, and two
labels for the same stock executable. Six alternating rotations per workload
ran on a shared 15-core macOS ARM64 machine. Values are median thread CPU in
milliseconds, with minimum and maximum in brackets.

| Fixed work | Stock A | Stock B | Direct allocation |
| --- | ---: | ---: | ---: |
| 500,000 log-field joins using UTF-8 text and list views | 45.386 [40.841, 47.164] | 43.057 [40.504, 46.666] | 26.802 [25.362, 27.915] |
| 1,024 singleton batches of 524,468-byte specs (16 repetitions of the 64-job input) | 175.856 [169.177, 222.173] | 177.243 [171.652, 181.115] | 139.451 [133.111, 163.049] |
| 16,000 dense batches of 128 specs | 406.668 [399.819, 471.119] | 409.767 [393.367, 422.736] | 359.612 [337.367, 410.495] |

The ordinary result used 38–41% less interval CPU than the two stock medians.
Both ordinary binaries also matched the SHA-256 and rolling checksum oracles
under `MESH_GC_STRESS=1`. The dense intervals overlapped at the tails.
Each dense repetition uses eight batches from the frozen 64-slot no-op profile.

A separate untimed probe called each release runtime with length-matched
Strings and counted `malloc`, `calloc`, and `memcpy` calls. Stock requested
two scratch allocations per tested join. Their combined requests were 269
and 330 bytes for the ordinary joins, 524,484 bytes for the singleton, and
156,287 bytes for the dense join. The candidate made zero scratch requests.
The probe saw the same final arena backing allocation on both sides.
Accounting for stock's inline separator stores, payload bytes copied per
join went from 282 to 141 and 372 to 186 for the ordinary joins, 1,048,936 to
524,468 for the singleton, and 308,478 to 154,239 for the dense join.
The candidate reads the slots twice to size and fill the result.

The exact 64-job input produced 64 singleton joins. A single pass saved about
2.2 ms in the assembly interval. The full local component process, including
parsing, spec construction, and the SHA-256 audit, had overlapping CPU ranges.
The archived durable producer's stock-to-stock CPU spread was 2.1–6.0 ms.
The component result leaves native submit-to-commit and queue throughput
unmeasured. [Prior producer measurements](../runtime-utf8-byte-length/mac-experiment/gate-01/measurements.json)

## Replay

Build release `libmesh_rt.a` from the stock revision in the receipt and
from that revision with the join patch. Use a compiler with the UTF-8
byte-length lowering. Run from the checkout with Python 3.11 or later, a C
compiler, and the native target triple.

```bash
python3 -Bu proof/runtime-string-join/replay.py \
  /path/to/meshc /path/to/stock/libmesh_rt.a /path/to/candidate/libmesh_rt.a \
  target/mesh-string-join-queue/replay --target aarch64-apple-darwin
```

The replay derives the spec builder and exact batch expression from the
frozen producer. It prepares the lists before timing and runs those batches
without PostgreSQL. It compares full output hashes with an independent
oracle, checks a rolling checksum in every timed iteration, and records
input, binary, and IR hashes. The receipt records the measured binaries and
gate results. Raw logs, the Mac allocation probe, and the Wasm check stay in
`target/mesh-string-join-queue`. [Receipt](receipt.json)
