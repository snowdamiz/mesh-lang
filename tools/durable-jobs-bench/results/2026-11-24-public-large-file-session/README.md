# Parked public LARGE file-path producer

This checkpoint covers the public trusted `b3557273d72be3802bf189b284e7caaf5f3caa0f` workload of 64 × 512 KiB jobs. No controller intake, reservation, or native launch was requested. The packet is parked and its controller entry points reject native execution. [Parked packet](controller-packet.json)

The existing public runner and migrator use ELF `af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355`. The experimental session source SHA-256 is `38cf5bc67dfe9a657ff7682226c73634eff9d9b78d735048776dbe7c0aac9577`. The diff adds one verb to the warmed session. It reuses the proven private-directory path, request ID, and file parser. Both verbs use the original transaction function, and the observer queries use that same connection. The file-path adapter requests the original normalized fixture after the inherited submit timestamp. It performs no fixture read or decode ahead of the request. Fixture generation and hashing stay where the original harness runs them. [Session diff](session.patch) · [Staged source and adapter](stage.tar.gz)

The offline plan check bound all seven lanes. The two inline duplicate controls and file treatment share one broker binary. Original Mesh inline, Go River, Elixir Oban and Rust Apalis retain their exact default-storage inventories. The public runner and migrator stay pinned in every Mesh lane. The diagnostic and timing replay require one TLS producer, 64 scalar enqueue calls, one enqueue transaction, 128 distinct application XIDs and 64 distinct terminal XIDs. The original Linux build deleted its runtime archive from the build directory on exit. Any resumed native link must recover the pinned archive or reproduce its exact SHA-256 before linking, and native timing requires passing diagnostics under the original watchdog. [Offline plan check](controller.py)

## Offline evidence at pause

- The uninstrumented file and inline modes and original stock inline all passed PostgreSQL 17.11 TLS correctness on the canonical fixture. Each persisted all 64 payload digests with the expected IDs and scalar boundaries. File mode passed out-of-scope path, symlink, malformed input, request-ID mismatch, explicit rollback, rollback after 62 scalar writes, and a committed submission whose reply the client did not consume. The retry returned the same 64 IDs in a fresh transaction.
- One earlier file-path correctness attempt reached the original 15-second reply watchdog while the producer transaction remained open. The later local-only replay allowed 90 seconds to finish correctness checks. The native plan retains the original 15-second watchdog.
- The diagnostic file-mode trace emitted all 64 × 524,470-byte singleton spec frames. Offline replay matched every digest against the earlier compiled capture for the same fixture file. The first checker had incorrectly assumed lexically sorted JSON fields. The raw trace and checker failure are both retained. The corrected trace-inline run is pending.
- The baseline audit re-scored all five default blocks and the first three completed EXTERNAL blocks. Public default medians were 1,814 ms to durable enqueue and 2,524 ms through settlement. Apalis led default submit by 719–1,035 ms in the five paired blocks. Oban led one block by 130 ms. River trailed all five. The two storage plans have different measured peer receipts and are separate.
- The earlier protected default pilot saved 298 ms to durable enqueue against the faster duplicate. The five protected Mesh EXTERNAL pairs saved 278–306 ms with stock peers and 72–257 ms with symmetric EXTERNAL peers. Ordinary V1 used direct-file on both Mesh lanes, and broker-once also compared direct-file paths. The old evidence motivates a test but supplies no saving for this public build. The spec-preparation component measured a different change over the same 64 frames and is retained separately.

[Local proof receipts](evidence/local-proof-receipts.tar.gz) · [64-frame file-path replay](evidence/trace-file-replay.json) · [Paired public and historical receipts](evidence/public-baselines.json)

## Resume offline

After resumption, rerun `python3 -B "$d/audit.py"` against the finished public series, where `d` is the result directory below. Review the separate default and EXTERNAL paired gaps before any controller decision. Run the pending untimed diagnostic against the pinned local compiler/runtime with these commands from the checkout root.

```sh
d="$PWD/tools/durable-jobs-bench/results/2026-11-24-public-large-file-session"
w="$PWD/target/2026-11-24-public-large-file-session-resume"
p="$PWD/target/2026-11-24-trusted-jobs-product-composition"
mkdir -p "$w/tmp" "$w/stage"
tar -xzf "$d/stage.tar.gz" -C "$w/stage"
export TMPDIR="$w/tmp" PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin
export PRODUCT_SOURCE="$p/source"
export PRODUCT_FROZEN_HARNESS="$PWD/target/enqueue-spec-preparation/ordinary-v2"
python3 -B "$d/audit.py" > "$w/public-baselines.json"
python3 -B "$d/build.py" "$p/local-fixture-sealed" "$p/cargo/debug/meshc" \
  "$p/cargo/debug/libmesh_rt.a" "$w/trace" --trace
python3 -B "$d/proof.py" "$p/local-fixture-sealed/mesh" "$w/trace/broker" \
  "$w/trace/broker" "$w/stage/adapter" "$w/proof-inline" trace-inline
```
