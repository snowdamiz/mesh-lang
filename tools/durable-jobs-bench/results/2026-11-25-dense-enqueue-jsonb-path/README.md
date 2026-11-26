# Private PG17 dense enqueue path extraction

The candidate replaces the nested JSONB lookups for `spec.unique`, `spec.replace` and `spec.queue` with constant two-element paths. PostgreSQL 17.11 descends through the original binary container with `#>` and `#>>`. The stock first-hop `-> 'spec'` constructs an intermediate JSONB value containing the whole specification. The candidate avoids that construction in the independent-policy check and the ordered queue insert. [PostgreSQL 17.11 JSONB accessors](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/utils/adt/jsonfuncs.c) · [Candidate SQL](candidate.sql)

The measured grouped-refill source archive contains the enqueue schema from `b3557273d72be3802bf189b284e7caaf5f3caa0f` byte for byte. The released fixed8+d388 receipt records eight bulk calls and zero scalar calls. The verbose generic MERGE plan already evaluates `x.value -> 'spec'` and volatile `now_ms()` at one source projection per row. Its INSERT contains two schedule expressions and one cast per other typed field. Moving the schedule expression to an eager projection can change which direct-SQL cast fails first. PostgreSQL's JSONB record decoder also uses a separate key lookup per output attribute. The candidate preserves the complete MERGE, including lazy casts, generated identity order and the per-row clock. The stock and candidate MERGE plans are byte-identical. [Exact source](source-deb2a3bc49fe.tar.gz) · [MERGE plan](evidence/stock-merge-plan.json) · [Stock queue plan](evidence/stock-queue-plan.json) · [Candidate queue plan](evidence/candidate-queue-plan.json) · [Released fixed8+d388 comparison](../2026-11-25-dense-fixed8-logical-factorial/PEER-RESULTS.md)

Prior width-1024 work changed the call count. Spec preparation changed Rust-side allocation and encoding. Ordered repeat targeted producer wire traffic. The R2 producer-boundary observer failures limit attribution from its whole-arm CPU comparison. Those results motivate investigating this path and provide no estimate for the SQL change. [Width-1024](../2026-11-24-dense-producer-1024/README.md) · [Spec preparation](../2026-11-24-enqueue-spec-preparation/README.md) · [Ordered repeat](../2026-11-23-ordered-enqueue-repeat/README.md) · [R2 boundary](../2026-11-24-r2-d2-producer-boundary/README.md)

## Untimed PostgreSQL 17.11 equivalence

Both disposable verified-TLS databases used the exact archived Mesh producer source, compiled locally for Darwin ARM64, and the real restricted producer role. For each version the broker submitted the frozen 1,024-job input, returned matching generated IDs and created 1,024 ordered jobs and request receipts through eight calls. The receipts and jobs match the original outer commit timestamp, and a committed product row carries the outer XID. The unusual compiled input includes omitted fields, explicit null options, nested JSON, non-ASCII text, escapes, repeated tags, maximum unsigned version, duration bounds, a large unsigned JSON number, three queues and a keyless job. The proof covers outer rollback, a fresh retry, same-key/same-spec receipt retry, changed-spec conflict, an unusable JSONB value after a prior successful call, legacy missing function, revoked execution, layout mismatch and the ordinary uniqueness writer. Both versions used the same existing scalar paths for those cases. The three ordered path edits leave the layout function, wake triggers and events, guards, namespace and savepoint fences, storage settings and caller protocol intact. [Qualification](evidence/qualification.json) · [Build identity](evidence/build.json)

Seventeen compiled invalid-input rows cover decoding and validation before the writer. The NUL JSONB savepoint case separately tests failure after the namespace fence. Thirty-eight direct-SQL cases compare exact SQLSTATE and primary message for failing inputs and ordered dispositions and queues for successes. They cover non-object specs, invalid policy values, absent and null fields, object and numeric queue coercion, numeric text, decimal and overflow failures, boolean casts, ignored invalid delay under an explicit schedule, arithmetic overflow and competing bad casts. The proof compares those cases and preserves the original strict expressions for every other field. PostgreSQL 17.11 on Linux remains the native screen target. [Stock raw proof](evidence/stock-raw.json.gz) · [Candidate raw proof](evidence/candidate-raw.json.gz)

## Replay

Run the host-free replay from the repository root.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-enqueue-jsonb-path
python3 -B "$d/replay.py"
```

To repeat the untimed proof, extract the archived source into a new directory under `target`. Build `mesh-rt` from it with Rust 1.99 and the frozen lockfile. Link the archived fixture with its `replay/link.sh`, an available compatible `meshc`, and that runtime archive. Compile the [existing scoped probe](../2026-11-24-dense-producer-1024/probe/main.mpl) with the same runtime and run `prove.py EXTRACTED_SOURCE BROKER PROBE NEW_EVIDENCE_DIR` with PostgreSQL 17.11 on PATH.

## Controller intake

The sealed request is for the sole Linode controller after existing reservations. Three lanes use the released fixed8ms+d388 configuration with the same ELF for migration, broker and two 32-slot runners, 32 hot application TLS sessions, CPUs 0 and 1, original clocks and frozen workload. Both duplicates keep stock SQL. The treatment installs the candidate as the existing function owner in a fresh migrated clone. Each arm warms up before the measured phase. The two blocks reverse stock A, stock B, projected. The existing factorial scorer reconciles the process identities, CPU, original ledger clocks, 1,024 jobs, 2,048 independent application writes and one durable enqueue. An additional post-run read checks eight writer calls, 1,024 writer rows and zero scalar work.

Advance only if both measured blocks save strictly more than 10 ms and the duplicate spread on submit-to-commit against the faster control, improve submit-to-settlement against both controls and keep commit-to-settlement within the larger of 10 ms, 2% or duplicate spread of the faster control. The controller exports independent warmup and measured raw archives and its release receipt. This screen has no local performance measurements.

```sh
python3 -B "$d/controller.py" check
python3 -B "$d/controller.py" replay
# Sole controller with registered packet and active grant
python3 -B "$d/controller.py" intake PACKET_SHA
```
