# UTF-8 byte length lowering

Mesh lowers a direct `Bytes.length(Bytes.from_utf8(text))` to a single load of the String's byte count on 64-bit targets. The change lives in product commit `ba8dab0381e8c8be12853faea9c1351c1ba42dde`. The baseline compiler and the candidate use the same `mesh-rt` sources as Jobs runtime commit `3f69b150`. The source and binary receipts pin the two builds and their shared runtime. [Source revisions](current/source.txt) · [Replay receipt](current/replay.txt)

## Semantics

`MeshString` has a C-layout `u64` length at offset zero, followed by that many UTF-8 bytes. On 64-bit targets, the runtime reads the same word as a `usize` to size the copy. A valid String allocation fits the header and payload within the target's maximum object size. Its temporary Bytes would store the same length and return that length as Mesh `Int`. A 32-bit conversion narrows the header through `usize`, so codegen keeps the runtime calls there. The LLVM target's pointer size controls the decision, including for cross-compilation. [String layout](../../compiler/mesh-rt/src/string.rs) · [Bytes conversion and length](../../compiler/mesh-rt/src/bytes.rs)

Codegen requires direct typed MIR calls to both runtime intrinsics. Locals, declared functions, explicit native bindings, and calls through first-class function values keep their existing dispatch. An imported user module named `Bytes` resolves to its own functions. The operand runs once in its original position and can perform effects or panic. `String.length` continues to count Unicode scalar values. [Codegen](../../compiler/mesh-codegen/src/codegen/expr.rs) · [Compiled regression](../../compiler/meshc/tests/e2e.rs)

The removed temporary is a leaf allocation. Its allocation may request a future GC safepoint, and dropping it reduces collection pressure. The fused path retains the operand's safepoints and reads the header immediately after the operand returns. The replay checks actor behavior with `MESH_GC_STRESS=1` and verifies that a panicking operand prevents subsequent output. Allocation exhaustion and GC timing depend on allocation pressure. [Actor allocation](../../compiler/mesh-rt/src/gc.rs) · [Panic fixture](fixtures/panic/main.mpl)

Native ABI 1 requires live Mesh-owned return values with the correct layout and extent. The replay covers a native String that contains multibyte characters and NUL. It also probes a sufficiently sized native allocation with malformed UTF-8 bytes. Both compilers return the same byte count in that probe, and the copy through the explicit native binding preserves the raw bytes. Null or dangling String pointers and forged extents are outside the ABI's valid inputs. On 64-bit targets the original copy already requires readable storage for the whole declared extent. [Native ABI](../../docs/native-packages.md) · [Native probe](fixtures/native/main.mpl)

## Results

The current replay runs ordinary Mesh and native ABI fixtures at O0, O1, and O2. It checks stdout, operand effects, panic, GC stress, and the calls that remain for escaping values, aliases, shadowed names, and explicitly bound C symbols. For the frozen Jobs producer, `collect_specs` loses one static conversion site. The component check regenerates the exact 64 × 512 KiB input and returns the same 66,607,436-byte total across the 127 accepted and lookahead checks. This replay collects no new performance estimate. [Replay receipt](current/replay.txt)

LLVM 21 on macOS verified the Linux x86-64 IR and emitted an ELF object with a direct header load. The Android ARMv7 baseline and candidate objects have identical SHA-256 hashes and retain both runtime calls. The cross-check covers IR and object generation. Native execution and PostgreSQL proofs use macOS ARM64. [Object excerpts](current/object-excerpts.txt) · [Toolchain](current/toolchain.txt)

The local workspace build, compiler and runtime suites, formatter, lint, fuzz-target build, laws, and pinned Z3 checks passed. PostgreSQL 17.11 passed compatibility, scoped roles, compiled durable Jobs, and the effect fence. The standalone `--effect-compiled` gate requires a GNU/Linux host. [Workspace workflow](../../.github/workflows/authoritative-verification.yml) · [Verification receipt](current/verification.txt)

The original Mac experiment copied 66,607,436 payload bytes across 127 temporary conversions, plus 1,016 bytes of requested Bytes headers. Six rotations per label put the isolated checks at about 3.6 ms for either baseline and 1.65 µs for the experimental patch. Producer submit-to-commit and CPU timings overlapped the variation between the two identical baseline arms. The producer effect remains inconclusive, and that experiment collected no post-commit queue score. [Original experiment](mac-experiment/README.md) · [Raw Mac measurements](mac-experiment/gate-01/measurements.json)

The archived README and manifests describe the earlier scratch experiment. This bundle carries the raw measurement receipt, provenance, and the original patch compressed to preserve the exact bytes. Historical manifests also name artifacts from that earlier run. [Historical checksums](mac-experiment/SHA256SUMS) · [Transfer manifest](mac-experiment/TRANSFER.sha256) · [Original patch](mac-experiment/utf8-length.patch.gz)

## Replay the pinned inputs

Build `meshc` from the two commits in [source revisions](current/source.txt) on the same 64-bit Unix host. Use one `libmesh_rt.a` for both sides. The current replay used Rust 1.99.0, LLVM 21.1.8, and `cargo build --locked -p meshc -p mesh-rt` with `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_INCREMENTAL=0`. Run from the checkout that contains this bundle. Use Python 3.11 or later and a fresh output directory.

```bash
python3 -u proof/runtime-utf8-byte-length/replay.py \
  /path/to/baseline/meshc /path/to/candidate/meshc /path/to/libmesh_rt.a \
  target/runtime-utf8-replay --target aarch64-apple-darwin \
  --llvm-bin /path/to/llvm-21/bin
```

On Linux, pass the native compiler target triple and use the matching runtime archive. The optional `--llvm-bin` argument adds the Linux x86-64 and Android ARMv7 object checks. The script prints the compiler, runtime, and cross-object hashes. Generated inputs, emitted IR, and build logs stay in the replay directory. The script uses local C and archive tools and skips database access.

Run the PostgreSQL proof with a native Mesh toolchain and PostgreSQL 17.11. The scripts create disposable loopback-only clusters.

```bash
unset MESH_TEST_DATABASE_URL
PG_BIN_DIR=/path/to/postgresql-17/bin \
  bash scripts/verify-postgres-jobs.sh --compatibility 17.11 --parity-driver-check --j6-roles
PG_BIN_DIR=/path/to/postgresql-17/bin \
  bash scripts/verify-postgres-jobs.sh --effect-fence
```

Verify the bundle from this directory with `shasum -a 256 -c SHA256SUMS` on macOS or `sha256sum -c SHA256SUMS` on Linux. To verify the preserved patch's decompressed bytes, compare `gzip -dc mac-experiment/utf8-length.patch.gz | shasum -a 256` to the original `utf8-length.patch` entry. The reserved Linode matrix remains pinned to its frozen candidate. [Historical checksums](mac-experiment/SHA256SUMS)
