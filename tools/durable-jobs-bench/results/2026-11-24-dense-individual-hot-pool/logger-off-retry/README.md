# Logger-off retry for the individual-settlement dense transfer

The original SETUP exited 1 at 23:30:34 UTC. Both runners reached a drained shutdown, but the frozen harness rejected the event-buffer report before it could write a complete arm. Runner 0 emitted 1,008 lines, including 502 attempt-start and 502 settlement records. Its logger sampled a 4,104-byte peak. Runner 1 emitted 1,048 lines with a 3,876-byte logger peak. Neither run produced a qualified timing arm. [Original raw export](../evidence/setup/raw.tar.gz)

The Jobs correctness fixture hardcoded `event_log:true`. The canonical dense harness leaves `JOBS_PARITY_MESH_EVENTS` unset, as in the earlier public 2×32 measurements. The [shared fixture](main.mpl) deletes only the hardcoded option and uses the product default with its logger off. Its SHA-256 is `97b8e75f8568191ac2b3a3160ce9c40d0eebc822ba2ec4661259510734816661`. Actor readiness uses these exact source bytes and qualifies its runtime separately. The original proof-on fixture and its evidence remain archived.

The single replacement SETUP runs behind existing bookings. It checks the retained source, pinned toolchain and runtime SHA-256 against the original Jobs proof. If the runtime archive was cleaned after that proof, it repeats the sealed offline runtime build and stops before linking unless the resulting archive matches byte for byte. The original target and source tree are retained. The new Jobs and teardown witness ELFs have distinct manifests. The replay requires that only the fixture main and ELF file hashes change. It runs all eight original semantic cases against the relink and its witness, including dropped claim and application receipts, checked cursor fallback, and normal and forced teardown.

Before release, both dense diagnostics must pass the original workload, slot, TLS, application-write, individual-terminal and protected admission assertions. The off-clock verifier rejects per-job event records, an active event buffer or drop counters, and incomplete shutdown. It runs on both setup diagnostics and every Mesh warmup and measured arm. Timing uses one new immutable ELF for hot and both checked controls. Their only behavior difference is the per-pool checkout option. The stock peers and six-rotation plan stay pinned to the original request.

## Controller commands

Run from the repository root. Register the byte-exact replacement request files under their new IDs and use the existing bookings. Publish the successful SETUP release as `evidence/setup/controller-release.json` in this directory before timing. The timing requests are build-free.

```sh
d=tools/durable-jobs-bench/results/2026-11-24-dense-individual-hot-pool/logger-off-retry
python3 -B "$d/retry.py" check
python3 -B "$d/test_retry.py"

# Sole controller, under the matching grants
python3 -B "$d/retry.py" setup
python3 -B "$d/retry.py" timing 1 2
python3 -B "$d/retry.py" timing 3 4 5 6

# After six complete rotations
python3 -B "$d/retry.py" replay > "$d/evidence/final-replay.json"
python3 -B "$d/../report.py" < "$d/evidence/final-replay.json" > "$d/NATIVE-RESULTS.md"
```
