# Preserve true bulk enqueue with EXTERNAL storage

Owner-approved EXTERNAL materially reduced producer PostgreSQL CPU on the compiled slow workload. The median was **93 ms**, down from **221–227 ms** across the duplicate true-batch DEFAULT controls and **214 ms** in the EXTERNAL scalar fallback. It beat both DEFAULT controls and the fallback in all six rotations. It beat both DEFAULT controls to commit in five rotations and to final settlement in three. The canonical payload and TOAST sizes were identical across the four arms.

## Layout change

The isolated `ce5e551d` hunk maps only `mesh_jobs.jobs.args` storage `x` (EXTENDED) and `e` (EXTERNAL) to the original digest value `x`. The expected hash and every other digest input are preserved, including the argument type and compression setting.

The benchmark uses the same previously linked V1 executable for production and worker execution in every arm. After the fixture migration, the table owner replaces only the layout helper on each compatible arm. The replacement preserves both functions’ OIDs and ownership, ACLs, definer flags, search path and volatility. The batch writer body is byte-for-byte identical. EXTERNAL fallback uses the original helper. The SQL overlay applies before warming the broker or starting the worker, on a disposable database. A later migration with the old embedded SQL restores the original helper.

[Exact source hunk](frozen/ce5e551d-external-layout.patch) · [One-function installer](frozen/install-layout.sql) · [Original binary identity](frozen/local-pins.json)

## Compiled slow run

Six rotations of four fresh PostgreSQL 17.11 clusters used the canonical 512 × 16 KiB input, one runner, 64 slots, eight application lanes, a 100 ms hold, verified TLS, synchronous commit, fsync, full-page writes, and a 68-connection cap. All 24 arms reconciled 512 argument hashes, original ordered enqueue receipts and fingerprints, 512 attempt commits, 512 independent effect commits, and 512 terminal receipts. Each arm reached 64 simultaneous committed holds. The compatible arms made eight true batch calls returning 504 rows and eight scalar calls. The original EXTERNAL guard returned eight fallback markers with zero batch WAL, followed by 512 scalar writes.

| Metric, ms | DEFAULT A median [range] | DEFAULT B median [range] | EXTERNAL bulk median [range] | EXTERNAL fallback median [range] |
| --- | ---: | ---: | ---: | ---: |
| Submit to commit | 472 [317–782] | 382 [204–541] | 211 [159–584] | 617 [370–998] |
| Producer backend CPU | 227 [178–239] | 221 [147–258] | 93 [80–137] | 214 [184–241] |
| Compiled producer CPU | 61 [49–67] | 59 [41–66] | 57 [47–77] | 81 [76–93] |
| Commit to last settlement | 1,435 [1,127–2,068] | 1,294 [1,012–1,479] | 1,279 [1,129–2,438] | 1,375 [1,121–1,590] |
| Submit to last settlement | 1,907 [1,444–2,849] | 1,688 [1,216–2,019] | 1,509 [1,288–3,021] | 1,913 [1,673–2,408] |

Against the better DEFAULT in each rotation, the paired producer-backend CPU saving was 105 ms [66–132]. The paired submit-to-commit saving was 76 ms [−166–291]. Against EXTERNAL fallback, the paired savings were 110 ms [93–136] in backend CPU and 349 ms [98–727] to commit. The measured producer CPU saving occurs with the same stored payload bytes under DEFAULT and EXTERNAL. PostgreSQL skips the compression attempt under EXTERNAL. The shared Darwin host had variable load, and the overlapping full-drain timing ranges leave its effect on native drain unresolved.

[Raw arms, clock windows, WAL, sizes and paired distributions](evidence/rotations/summary.json) · [Canonical input and caps](frozen/canonical-input.json)

## Scope and rollback proofs

The original compiled proof suite runs with EXTERNAL already installed on both the legacy and qualified templates. It covers old/new overlap, request replay, uniqueness and replace, trigger visibility and suppression, RLS, schema version, ACL and isolation fallback, outer and savepoint rollback, truncated batch reply, and lost COMMIT reply. The expanded controls test the argument type and compression, MAIN and PLAIN, sibling column and table storage, added column, index and constraint, TOAST options, FORCE RLS, effect-key and wake/claim function definitions, and producer rejection on owner-only DDL. The added mutations require one fallback marker, zero batch WAL and 16 scalar rows on the real compiled producer.

A separate probe runs rollback and retry on the exact 512-job timed V1 binary. The rollback leaves zero product, job and receipt rows. Repeating the same keys then writes one durable product transaction and 512 jobs and receipts. The short compressible control also confirms that EXTERNAL forgoes compression while retaining true batching.

[EXTERNAL compatibility proof](evidence/external-proof/PASS) · [512-job rollback and retry](evidence/slow-rollback/receipt.json)

## Native crossover

The matched design retains both DEFAULT and EXTERNAL for Mesh, River, Oban and Apalis on the same pinned stock Linux PostgreSQL 17.11 binary and extension. Mesh gets the same compatibility helper in both arms. The three peers use their existing executables and the column policies are verified on empty tables before submit. Every arm verifies physical compression, bytes and terminal data afterward. The same CPU mask, input, workload and durable application writes apply to every lane.

The local overlay smokes are functional proofs for all eight lanes. The controller queue receives the sealed native SETUP request. Native runs require a separate controller grant and Linux receipts. The earlier LZ4 packet and native default baselines remain separate.

[Local peer smokes](evidence/local-peers/summary.json) · [Frozen native plan](frozen/native-config.json) · [Controller request](request.json)

## Replay

Run from the checkout root. The offline timing replay uses the archived ledgers, the exact input, and the raw counters. It requires no local PostgreSQL instance.

```sh
result=tools/durable-jobs-bench/results/2026-11-23-slow-external-compat
work=target/ordinary-slow-storage-transfer/external-compat
python3 -B "$result/replay/local_series.py" replay "$work"
python3 -B "$result/replay/verify_local_peers.py"
python3 -B "$result/replay/native.py" check
(cd "$result" && shasum -a 256 -c SHA256SUMS)
```

For fresh local execution, extract the harness and V1 binary archives into `$work/base` and `$work/artifacts-v1`. Extract the proof harness into `$work/source`, the proof binaries into `$work/artifacts`, and create `$work/tmp`. Set `PG_BIN_DIR` to PostgreSQL 17.11. The proof commands accept the work directory and a fresh evidence output directory.
