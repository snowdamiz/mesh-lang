# Public LARGE file-session continuation

The untouched parked packet documents the earlier attempts, including the original watchdog failure. This continuation compares the two producer transports under the public default-storage recipe for 64 jobs with 512 KiB payloads. The EXTERNAL series serves as a separate historical audit.

The file transport sends the already-generated private fixture path to the warmed broker after the inherited submit timestamp. The broker reads and parses that fixture and calls the same transaction function, using the same connection as its observer. The change removes the wrapper's fixture decode and re-encode and the transfer of the resulting full inline request over the producer pipe. The setup retains fixture generation, hashing, scalar SQL, and the runner and migrator ELFs.

## Offline evidence

The current audit reproduced five retained default blocks and five retained EXTERNAL blocks. The local compiler, runtime, and runner from the parked instructions were absent. The trace replay therefore used a Darwin rebuild from the sealed public source with the original Rust and LLVM versions. The newly linked broker used identical traced Mesh source. The untimed TLS PostgreSQL 17.11 replay emitted all 64 singleton spec frames at 524,470 bytes apiece. Every digest matches the retained file-path trace. The fixture binding records the raw public Mesh receipt for each of the ten audited blocks. [Audit](evidence/public-baselines.json) · [Trace receipt](evidence/trace-inline.json) · [Fixture binding](evidence/trace-binding.json) · [Local broker provenance](evidence/local-trace-broker.json)

## Controller commands

Run from the checkout root. The published request in the controller queue must byte-match the [operational intake](intake.json). Only the sole controller runs the setup and timing commands after earlier assignments release.

```sh
d=tools/durable-jobs-bench/results/2026-11-24-public-large-file-session/resume
python3 -B "$d/controller.py" check
python3 -B "$d/slot.py" setup
python3 -B "$d/slot.py" timing
python3 -B "$d/controller.py" scores "$d/evidence/native/setup" "$d/evidence/native/timing"
```

SETUP scans retained archives for the original SHA-256 and builds from the sealed Linux source if needed. If the rebuilt archive differs, SETUP binds a separately labeled stock-source inline relink through the same compiler and rebuilt runtime as the treatment. In that case all five timing blocks add the relink lane and retain the original stock inline reference. Both branches retain the two inline duplicates on the treatment broker ELF. SETUP must pass the original Linux 15-second watchdog, TLS and scalar enqueue checks, explicit and late rollback, and the same-ID retry after a committed reply was left unread. The timing launcher requires a successful SETUP release and replays qualification from the raw archive before dispatch.

The five interleaved blocks preserve the original default-storage River, Oban, and Apalis inventories. Scores report submit to first durable commit, submit through last settlement, and commit through last settlement. The scorer revalidates both phases and preserves each measured raw receipt hash. The faster inline duplicate bounds the claimed saving from the producer transport. Peer comparisons use the file-treatment lane.

## Native release

The first SETUP connection expired after the Mac slept. The controller recovered the completed Linode export without replacing the failed transfer. The rebuilt Linux runtime matched the original SHA-256. A fenced readback verified all seven stock manifest members and the copied broker inputs after the first checksum command skipped the member lines. [Recovered qualification](evidence/native/setup-export-recovered/qualified.json) · [Member readback](evidence/native/setup-export-recovered/member-readback.json)

The first timing inventory pointed its experimental lanes at the parked adapter path. Those raw receipts remain in the original timing directory. The separately sealed path-only retry matched the installed adapter paths and reused the qualifying SETUP. All five blocks reconciled, with 64 completed jobs per arm across 35 warmup and 35 measured arms. [Timing retry](adapter-path-retry/intake.json) · [Timing release](evidence/native/timing-adapter-path-retry-release.json) · [Scores and raw receipt hashes](evidence/native/score-replay.json)

The file path saved a paired median 290 ms to durable enqueue (188–354 ms) and 288 ms through settlement (178–389 ms) against the faster same-image inline duplicate. It finished ahead of Oban and River in all five pairs. Apalis led four pairs and retained the fastest end-to-end median. Its 26.776 s block-four result included three jobs whose effects committed within 89 ms of enqueue commit and whose settlements occurred together at 25.282 s. All 64 jobs in that arm succeeded on their first attempt. The exact cause of the settlement delay is unverified.

Regenerate the scores offline from the checkout root.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-24-public-large-file-session/resume/adapter-path-retry/retry.py scores
```

The controller verified removal of the disposable Cargo, scratch and rebuild trees while retaining the pinned archive, broker, adapter and receipts. [Cleanup receipt](evidence/native/timing-adapter-path-retry-cleanup.json)
