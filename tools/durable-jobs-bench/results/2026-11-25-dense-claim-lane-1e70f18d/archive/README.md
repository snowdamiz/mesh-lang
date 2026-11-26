# Confirmed grant launch on the split claim lane

The private patch starts from public `1e70f18d624a841740d3bbb3d0fd09b269257f68`. It enables early launch with `MESH_JOBS_CLAIM_LANE_LAUNCH=1` for ordinary trusted shared-process runners with the existing split set-based claim path. The canonical logger-off fixture, producer, two-write handler, and individual terminal SQL are pinned. The patch keeps the original thread and connection roster.

After checked grant COMMIT has returned idle, the claim thread converts the confirmed reservation into active jobs under the runner state lock. Each job goes through the existing launch and registration checks, including the lease deadline, global or queue stop, pause and current slot limit. A zero-slot reservation still fences shutdown and the next request until the dispatcher adopts the attempt handles. The dispatcher retains exclusive lifecycle I/O and releases a slot only through the existing settlement path. The scoped claim thread joins as part of dispatcher shutdown.

The historical asynchronous writer moved lifecycle I/O to a writer thread and could share a terminal transaction among several jobs. Its held-receipt proof isolated overlap in that design. The earlier strict admission and settlement pipelines kept distinct terminal transactions but used different grant plumbing, binaries or separate worker executables. This patch changes confirmed-grant delivery on the exact public trusted shared-process path and leaves the scalar terminal statement and transaction cardinality in place.

## Untimed proof

The PostgreSQL TLS relay holds the first handler's second write receipt, waits for a separate grant transaction's idle receipt, then allows the first job to reach its individual terminal commit. It withholds that terminal ReadyForQuery after PostgreSQL has committed the row and forwards the peer's grant ReadyForQuery while the terminal receipt is still held.

With the switch on, the peer reaches its first application write while both the blocked attempt and the peer are charged. With the switch off, the original attempt remains charged and the peer stays pending. The two successful jobs have four distinct application XIDs and two distinct terminal XIDs, separate from their enqueue and grant XIDs. The same sequence with a dropped terminal receipt yields one stale confirmation on the original token and one execution of each handler. The SIGTERM case drains the original attempt and refunds the unstarted peer. The local-control witness separately tests pause and scale-down between reservation and confirmed delivery. Both retain the peer for refund without entering the handler.

The grant-edge fixtures verify cancellation before grant and an expired or malformed receipt for a committed grant. The first expired or malformed grant never enters; a subsequent confirmed attempt settles. The original semantic suite checks unknown grant COMMIT, ambiguous application receipts, checked cursor fallback, and normal and forced actor teardown. The held tests require a stopped, drained report with zero tracked jobs and reservations.

The diagnostics switch `MESH_JOBS_GRANT_DIAGNOSTICS` records the client-side idle receipt wall clock and two monotonic ages for each job. It marks scheduler submission immediately before `spawn_owned` and entry immediately before invoking the handler. Samples are appended after the handler returns and written once after the claim lane and attempts drain. The SQL stamp-to-receipt interval includes grant execution, commit and receipt transport. Setup diagnostics join all 1,024 recorded job IDs back to the native ledger. Warmup and measured arms leave diagnostics and per-job event logging off.

## Native handoff

`setup.json` and `timing.json` are submitted byte for byte in the existing controller inbox. The controller owns assignments, Linode access, host fences, export and release. The retained `native-stage.tar` is the exact setup payload. The setup builds a private runtime with the pinned native toolchain, relinks the canonical fixture, runs the untimed proofs, and qualifies one diagnostic per switch setting.

The build-free pilot reverses the full lane order. All three Mesh arms use one native ELF and hot checkout. The legacy labels `checked-a` and `checked-b` are duplicate early-launch-off controls in this campaign. River, Oban and Apalis keep the existing native inventories. A candidate must clear the larger of the duplicate-control spread and 5% of the faster control on both submit and enqueue-commit clocks, and finish its last application write earlier, in both rotations. A miss closes the candidate. An incomplete run produces no result. There is no automatic follow-up campaign.

```sh
python3 -B campaign.py check
# Sole controller only, under the matching active assignments
python3 -B campaign.py setup
python3 -B campaign.py timing 1 2
python3 -B campaign.py replay
```

## Related evidence

- [Historical asynchronous writer and held-receipt proof](../../../tools/durable-jobs-bench/results/2026-11-23-ordinary-completion-writer/README.md)
- [Historical same-ELF writer timing and terminal XID change](../../../tools/durable-jobs-bench/results/2026-11-23-ordinary-completion-writer-native-repair/README.md)
- [Earlier strict admission and separate-worker pipeline](../../../tools/durable-jobs-bench/results/2026-11-23-admission-v3/README.md)
- [Public native hot baseline and independent claim-to-entry audit](../../../tools/durable-jobs-bench/results/2026-11-25-dense-claim-to-entry/README.md)
