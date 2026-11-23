# Mesh scheduled-work index range

The candidate changes the PostgreSQL due-time query shared by listening Jobs runners. It reads the same materialized clock through a scalar subquery, which lets PostgreSQL use `scheduled_at_ms` as a range condition on `mesh_jobs_queue_due`. The candidate is `ee59322673e9fb5fc861e4a53781a70840adbcde` on local branch `t3/native-jobs-due-range`. [Source patch](runtime.patch)

A fresh PostgreSQL 17.11 plan check returned the same result for the baseline and candidate SQL. The query returned no delay for a ready-only backlog, and it selected the eligible retryable job at 2,000 ms when earlier ineligible work and later scheduled work were present. The ready-only index scan emitted 1,024 rows for the baseline and zero for the candidate. With future work it emitted 1,026 and two respectively. This is a query-plan result. The separate trace-off repeat measures queue throughput. [Plan witness](plan-witness.json)

The candidate build uses the baseline compiler and frozen worker sources with a release runtime rebuilt from the local candidate revision. `build.sh` stages the candidate under the matrix owner's host directory. `run-candidate.sh` calls the same native arm driver. The archived baseline and the dense profile remain the reference evidence. [Baseline](../2026-11-23-baseline/README.md), [dense diagnostic](../2026-11-23-dense-diagnostics/README.md)

```sh
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin \
    python3 -B tools/durable-jobs-bench/results/2026-11-23-mesh-due-range/verify-plan.py
```

The exact candidate passed the macOS workspace suite with 4,813 tests, the PostgreSQL 17.11 compatibility and scoped-role gate, and the effect-fence proof. The native Linux dense and sparse diagnostic arms reconciled 1,048 jobs with two ordered writes per job. [Local proof receipt](local-proof.json), [full native repeat](../2026-11-23-due-range-repeat/README.md)
