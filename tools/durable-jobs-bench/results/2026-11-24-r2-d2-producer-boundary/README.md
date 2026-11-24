# Short R2 D2 producer boundary

The five measured 128-job Linode arms each used one guarded enqueue_batch_v1 call, zero scalar calls, one durable enqueue transaction, and 256 independent application writes. Their submit-to-enqueue-record window was 36.31–47.90 ms. The paired gap was 19.18–33.44 ms versus River and 23.71–36.36 ms versus Apalis. Mesh’s recorded commit-to-caller-acknowledgement interval was 2.66–3.42 ms, outside that producer gap. The replay checks raw archive hashes, the original broker and exit-confirmed runner identities, and successful durable reconciliation. River and Apalis record an adapter return clock. Mesh and Oban record caller acknowledgement. [Five-block raw audit](evidence/offline/audited-peer-scores.json)

The prior native receipts capture submit, transaction commit timestamp and acknowledgement. Their writer counters prove the selected bulk call, with no per-stage timing. The separate booked enqueue-spec-preparation experiment extracts the exact 154,241-byte frame and covers decoding, validation, normalization, JSON conversion, encoding and fingerprinting. It owns the preparation decision, including any subsequent conditional broker trial. The older scalar packing experiments used a different SQL path. [Preparation packet](../2026-11-24-enqueue-spec-preparation/README.md) · [Original R2 timing](../2026-11-24-exit-confirmed-bulk-d2/native-results.md)

## Controller experiment

The five groups rotate four lanes on the pinned Linode using PostgreSQL 17.11 and CPUs 0 and 1. Both controls reuse the untouched prior bulk broker, migrator, workload, runner and adapter. The client and nested lanes relink the exact stock producer source with a benchmark-only recorder. They keep the original stock migrator, direct exit-confirmed R2 D2 runner, TLS roles, capacity, wakeups and success_cohort policy. The already qualified peers and their scores are pinned to their original raw blocks.

The caller records fixture file read, JSON decode, frame formation and broker pipe handoff. The broker buffers fixed marks for input receipt through first send, each PostgreSQL command’s write and ReadyForQuery, first bulk DataRow, commit response and receipt publication. The existing outer acknowledgement includes a separate receipt validation and PostgreSQL commit-timestamp lookup. The reducer prices that explicitly. A bracketed wall/monotonic conversion places PostgreSQL’s recorded timestamp on the producer timeline. The record timestamp can precede the COMMIT request’s client write return. Confirmed durable success arrives at ReadyForQuery. The intervals close to the original submit-to-acknowledgement clock without adding nested SQL timers or overlapping pipe-write time. The broker’s elapsed interval between the product INSERT response and SAVEPOINT send includes spec work and remains unallocated here.

The nested lane enables nested statement and PL function statistics on the disposable database’s real producer role. The guarded query’s top-level time is inclusive. Layout and the writable CTE are subintervals. The CTE combines MERGE and receipt insertion, with trigger time already included. Function self and total times give independent visibility into layout and wake work. The report retains all nested rows and counts intervals once.

The reducer reports both control spread and paired observer deltas. An observer is usable when the absolute value of its median shift is at most 2 ms and its paired shift is within the larger of 1 ms and the duplicate spread in at least four groups. A pre-commit interval is material when its median is at least 5 ms and clears median control noise, with four of five groups also clearing the per-group bounds. Material elapsed time still needs a verified, avoidable cause before selecting a change. A candidate would need the same native producer contract, independent application commits, and request-key replay, schema, ACL and DDL fallback and conflict proofs. Candidate selection is pending native attribution.

## Replay

From the repository root, run:

    d=tools/durable-jobs-bench/results/2026-11-24-r2-d2-producer-boundary
    python3 -B "$d/offline.py"
    python3 -B "$d/controller.py" check
    (cd "$d" && shasum -a 256 -c SHA256SUMS)

The sole controller owns setup and timing under the existing three locks. The sealed request gives the exact commands and expected receipts. SETUP proves the relink and both clock domains using a diagnostic full native arm with the nested observer. It is excluded from the timing result. The first two opposite-order TIMING groups must reconcile before the last three. Every dispatch has a distinct evidence path, including incomplete and failed attempts. [Controller request](request.json) · [Offline checks](evidence/offline/checks.json)
