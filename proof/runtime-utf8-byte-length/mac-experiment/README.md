# Elide the temporary Bytes in a UTF-8 byte-count expression

The local compiler experiment is based on Mesh commit `dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17` (tree `6aae4c7638ed1166b18223bb3a1059a620aed8e4`). Its only compiler edit is [utf8-length.patch](utf8-length.patch), SHA-256 `0a106045ec55dd41738ed100a872244f557d9d64f06f7c5fac031bc3352c0468`. The builds used an archived copy of the base in `src/`. Concurrent edits in the active checkout stayed outside both builds.

The prior-work check covered local refs, compiler call lowering, and existing performance artifacts. The pointer-free GC change in `6ae02449` still allocates and copies in `mesh_bytes_from_utf8`. The earlier ordinary code-size experiment changed compiler build settings. Neither implemented the composition.

## Behavior

The LLVM lowering recognizes a direct `Bytes.length(Bytes.from_utf8(text))` in typed MIR. It checks both resolved runtime names, signatures and arities, and excludes local functions, user functions, and native bindings from the match. It evaluates `text` once and loads the String header's 64-bit byte count. The runtime already uses that word to size the copy. The new path inserts no allocation or safepoint between the operand result and the load.

The focused gate compares executable output from both compilers at Mesh optimization level 2. The ordinary program covers empty text, precomposed and combining Unicode, codepoint length, a side effect evaluated once, a local shadow, aliases, and a 524,292-byte string. A separate module shadows the builtin module. The C fixture passes a String containing NUL through `mesh_string_new`, returns native Bytes using `mesh_bytes_new`, borrows escaping Bytes in C, and declares a native binding directly to the existing conversion symbol. Actor variants also execute under `MESH_GC_STRESS=1`. The IR checks confirm the direct load and preserve the calls for explicit native bindings and escaping values. The ordinary alias and local-shadow function bodies are identical before and after.

The [AArch64 disassembly](evidence/candidate-collect_specs.asm) shows a header load in the candidate's `collect_specs`. The baseline calls the constructor and length. Its linked length symbol shares an address with `mesh_tuple_size`, so the disassembler displays the latter name. [Baseline symbols](evidence/baseline-collect_specs-symbols.txt)

## Frozen workload and copy cost

The frozen producer's `main.mpl` is byte-for-byte identical to the source under `target/jobs-perf-work/frozen-fixture` and the tracked integrated-uncut proof fixture. Its SHA-256 is `0c72aaa2f0f34c49d07fd2f6cad2d39f491c0857897a0ac7a0facb6b85b4464d`. The 64 × 512 KiB input totals 33,554,432 payload bytes in one submitted transaction. The batch file SHA-256 is `30f91354336fab21dbe85f2933f3347ec53ab7634af3805169a892536747fa1c`. The harness case SHA-256, computed over the ASCII batch digest, is `9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1`.

Each encoded spec is 524,468 bytes. One spec plus brackets fits in the 1,048,576-byte limit. Two specs plus their comma and brackets need 1,048,939 bytes. Each collection starts with an empty list, accepts one spec, and tries the next spec before returning its accepted prefix. The rejected spec is re-encoded and checked again on the next iteration. A diagnostic copy of the source recorded the check index, encoded length, running batch size, and each enqueue batch size. Both compilers produced exactly the same sequence. There is one static composition site, with 64 accepted and 63 rejected-lookahead evaluations, 64 one-row enqueues, and 127 dynamic length checks. The baseline copies 66,607,436 bytes of spec data and requests 1,016 bytes of Bytes headers for these checks. The candidate replaces those 127 copies and length calls with header reads. Both arms still build the same specs and enqueue the same input. The replay uses the frozen dense-mode path. The fixture's separate `producer_commit` fault probe requires the input to fit a single enqueue batch.

## Local measurements

The timed producer used the uninstrumented, frozen Mesh source. Both release compilers used one release runtime archive and the same local native identity archive, rebuilt from the pinned C source for macOS ARM64. Both baseline labels invoked the same executable. The host had 15 logical CPUs and a one-minute load of 22.47–24.55 during the producer trials. The owned loopback PostgreSQL 17.11 instance used verified TLS, synchronous commits, fsync, full-page writes, and commit timestamps. Enqueue used a fresh database per arm, with migration and reconciliation outside timing. There were six rotations covering every ordering of A, B and C, after one warmup per label. There were zero failed timed trials. Each timed arm verified one product commit, 64 receipt IDs, 64 job payload hashes and the original arguments. Each of the 64 per-enqueue savepoint XIDs had the same durable commit timestamp as the product row.

The component probe used the frozen `job_spec` function to prepare 64 real encoded strings before starting its clock. It then traversed the observed 127 index visits and checked the byte-count sum. This prices measuring and temporarily copying the already encoded specs. The probe uses a prebuilt list, so it omits re-encoding, database I/O and producer startup.

Values are medians with ranges over six trials per label.

| Measurement | Baseline A | Baseline B | Candidate C |
| --- | ---: | ---: | ---: |
| Isolated 127 checks, ms | 3.631 [3.498–4.368] | 3.580 [3.338–4.017] | 0.00165 [0.00150–0.00200] |
| Submit to durable commit, ms | 1,321 [864–2,160] | 1,253 [854–2,173] | 1,143 [804–1,846] |
| Producer process CPU through reply, ms | 171.7 [149.5–178.0] | 171.3 [151.0–184.1] | 165.6 [148.6–172.6] |

The isolated cost was visible in all six rotations. The candidate's per-rotation producer CPU delta versus the mean of the two controls ranged from 7.5 ms slower to 10.8 ms faster. Identical baseline labels differed by up to 285 ms in submit-to-commit. The candidate was faster than their mean in five rotations and slower in one. The producer impact is inconclusive under this load. The component result accounts for about 3.6 ms per submission in this setup. Its timer starts after spec construction and excludes any allocation-pressure effects on the rest of the producer.

Submit-to-commit starts immediately before the parent invokes the enqueue executable and ends at the PostgreSQL commit timestamp of the product row. It includes process launch, reading and parsing the input, connection setup, encoding, and transactional work. The original queue throughput denominator in `scripts/jobs-parity.py` runs from first durable enqueue commit to last durable settlement. The pickup clock also starts at commit. This experiment measured the producer with zero workers and leaves the postcommit queue score unmeasured. [Raw timings and reconciliation](gate-01/measurements.json)

## Run the focused gate

Use Python 3.11 or later, LLVM 21 and the repository's normal Rust, C and linker tools. Resolve all output paths inside the scratch directory. The local compilers, the runtime archive, the pinned fixture files, and the check programs are listed in [SHA256SUMS](SHA256SUMS).

```bash
r="$PWD/target/linode-utf8-length"
bash "$r/focused-gate.sh" "$r/bin/baseline" "$r/bin/candidate" \
  "$r/bin/libmesh_rt.a" "$r/frozen-inputs" aarch64-apple-darwin "$r/gate-replay"
PG_BIN_DIR=/path/to/postgresql-17.11/bin PYTHONDONTWRITEBYTECODE=1 \
  python3 "$r/measure.py" "$r/gate-replay" 17.11
```

The focused gate expects the pinned source archive unpacked at `$r/src`. It rebuilds all fixtures with both compilers. The measurement script uses a fresh loopback-only database cluster under the supplied run directory and disposes of it on exit. All producer and verification artifacts are scoped to this scratch directory.

## Local verification

The workspace build, core test suite, runtime and library tests, compiled integration tests, fuzz-target check, format and lint checks, law tests, and pinned Z3 checks passed. The PostgreSQL 17.11 compatibility, scoped roles, compiled durable Jobs, and effect-fence proofs passed. The PostgreSQL proof was repeated with the scratch repository's Git metadata pinned to the tested source. Its first run inherited the enclosing checkout revision in the log header. A source audit checked all 1,504 tracked blobs against the pinned commit and the single compiler patch. The proof scope is Darwin ARM64 and loopback PostgreSQL 17.11. [Workspace exit codes](evidence/workspace-status.txt) · [PostgreSQL exit codes](evidence/postgres-verified-status.txt) · [Source audit](evidence/source-identity.txt)

## Transfer for a future same-host native comparison

1. On an explicitly authorized native benchmark host, put the patch, `base.sha`, gate scripts, fixture sources, and `frozen-inputs/` in a new owned scratch directory. Verify [TRANSFER.sha256](TRANSFER.sha256) from that scratch directory before rebuilding. Materialize `git archive "$(cat "$r/base.sha")" | tar -xf - -C "$r/src"` with an empty `src/` directory.
2. Build the baseline release compiler and runtime from that source with pinned Rust and LLVM 21. Save separate copies and their hashes before applying the patch with `git apply --check` and `git apply` from `$r/src`. Rebuild the candidate compiler with the same flags. Keep the original runtime archive fixed across both arms. If transferring to a later source commit, check the patch there and rebuild and gate both arms from that new base.
3. Run the focused gate with the effective native triple. On GNU/Linux ARM64 it reuses the exact frozen manifest and native archive. On another architecture, pin a corresponding build of the same identity C source and use it for both arms. Hash the actual inputs, compilers, emitted objects or IR, link settings, and binaries. Keep output paths and any layout policy matched across arms.
4. Replay submit-to-commit in a maintenance window with six interleaved permutations of two byte-identical baseline labels and the candidate. Use the frozen case hash and PostgreSQL commit timestamp. Keep candidate diagnostics outside timed rotations.
5. For a separate queue score, use the normal native runner topology, versions, database settings and trusted parity reducer. Reconcile all 64 job payloads, attempts, effects, terminal commits, and existing parity criteria. Preserve the submit-to-commit column separately from the original first-commit-to-last-settlement score.

This experiment used only the local Darwin host and its owned loopback cluster.
