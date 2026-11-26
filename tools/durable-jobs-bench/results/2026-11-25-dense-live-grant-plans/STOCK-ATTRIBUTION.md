# Stock dense grant attribution and closeout condition

Same-statement TID access is closed. The existing stock-only reservation uses the public grouped-hot `b3557273` executable, with 1,024 jobs and two 32-slot runners. Its fresh dispatch uses the unchanged `1f5acbee` recovery packet. The old pre-dispatch failure and the original successful PostgreSQL 17.11 SETUP remain in the record. [Stock packet](procfd-recovery/controller-packet.json) · [Working-directory replay](procfd-recovery/evidence/launch-cwd/README.md)

## Stock result

The fresh reservation released exit 1 at 02:49:51 UTC. Group 1 reconciled all three 1,024-job arms with zero failed jobs, and a local replay matched its retained reduction exactly. The group 2 sampled arm failed the equality assertion between the stock TLS census and the pinned roster at readiness. The saved stock census contained six participant TLS backends, 32 application backends, and one producer. The later roster validation required eight participant backends and passed before the equality assertion. The later roster itself was not retained. The arm produced no grant plans, and the final control did not launch. [Fresh release](procfd-recovery/evidence/native/controller-release.json) · [Group 1 replay](procfd-recovery/evidence/native/blocks/01-noop/reconciled.json) · [Group 2 failed arm](procfd-recovery/evidence/native/blocks/02-noop/sampled/measured-mesh/raw.tar.gz)

All 25 in-window drain grant plans match nested query ID `6827047345048828466` under `parity_subject` in `pg_stat_statements` (user OID 16421, database OID 16424, `toplevel=false`). Its normalized SQL is the stock `WITH picked ... granted` statement containing the jobs UPDATE. Both runners are represented, with 13 and 12 samples from their pinned grant backends.

- **UPDATE target** used a plain `jobs_pkey` index scan in all 25 samples. The index condition constrained only namespace. Each scan emitted 1,024 rows in one loop, for 25,600 emitted rows and 27,487 shared buffer hits across the samples. The parent applied the ID equality as a join filter. Zero samples used a bitmap for the UPDATE target.
- **Ordered pick** used `mesh_jobs_ready` and emitted 4–16 jobs per sample, 320 rows in total. The picked subplans recorded 1,295 shared buffer hits. These counts cover the same 25 executions. [Sampled raw archive, member `adapter/grant-plans.json`](procfd-recovery/evidence/native/blocks/01-noop/sampled/measured-mesh/raw.tar.gz)

The flush-checked sampled bracket recorded 98,138 `jobs_pkey` heap fetches out of 121,528 for the jobs relation. The duplicate controls recorded 102,067 of 128,438 and 107,233 of 134,331. These are relation and index totals over all statements, including observation and shutdown-tail work. The sampled plans show where the broad UPDATE work occurred. The counters do not assign every historical fetch to that node.

The sampled arm took 170.6 ms longer from submit and 167.9 ms longer from commit than the duplicate-control means. Duplicate spreads were 16.3 ms and 13.3 ms respectively. Use these differences as observed profiling overhead in this rotation. They do not estimate production cost or the savings of changing the UPDATE. The specified bitmap condition did not open.

## Condition for a separate design

Consider a separate two-command design only if real stock auto plans on the live, churning table show the expensive UPDATE-target bitmap often enough to establish a material opportunity. Compare the UPDATE-target access path separately from the ordered selection. Report sample coverage and observed visits and buffers in each arm without treating bitmap index entries as fetched heap tuples or extrapolating elapsed cost from sample frequency.

That design would lock, select and render the manifest, then issue a bounded UPDATE under a fresh command snapshot within the same locked READ COMMITTED transaction. It must fully verify the updated row count and row identities against the manifest. Before any native booking, it must pass the exact EvalPlanQual (EPQ) race, mixed-version and ordering tests, and all fallback, clock and transaction checks. The archived race advances the first picked row to a new version between the reader’s command snapshot and its row lock. It covers both an empty return and a partial grant that skips the first row. This is a decision condition for a separate design. [Exact EPQ race and closed treatment](../2026-11-25-public-dense-tid-grant/README.md)

The earlier real workload recorded 147,218 and 146,252 jobs-table index fetches. Nested query `6827047345048828466` accounted for 131,661 and 132,080 shared-buffer hits in the respective brackets, but its SQL was redacted and its plans were absent. Seeded stock auto plans used a primary-key probe at sixteen slots and repeatedly built a broad bitmap at eight slots. Those records establish the mechanism on the fixture. They provide no live UPDATE-target frequency or heap-fetch attribution. [Real trace results](../2026-11-24-public-dense-set-claim/NATIVE-RESULTS.md) · [Seeded stock access paths](../2026-11-25-dense-grant-index-entries/README.md)


## Reproduce the retained result

From this directory, run `python3 -B replay-stock-capture.py | cmp - evidence/stock-capture-replay.json`. It replays the three group 1 arms from the raw archives and verifies the exact overhead clocks, grant plan identities, the completed group 2 control, and the failed sampled arm. It preserves both exit-1 releases and the incomplete second-group decision. The sole controller archived all five attempts in `1453970f9`. [Exact replay receipt](evidence/stock-capture-replay.json)

The sole controller and experiment owner closed the stock capture with the evidence above. A second-group-only retry would add independent replication, but was not needed for this decision and has no reservation. The successful first group remains the evidence for this scope.
