# Cap-71 Mesh SQL audit retry

The first native Mesh SETUP stopped in the audit before the workload was submitted. PostgreSQL returned SQLSTATE `42703` because the statement filter used bare `CURRENT_DATABASE` where it needed `CURRENT_DATABASE()`. PostgreSQL logged the exact broker-wrapped SQL and pointed to the unbound identifier. The failure archive contains the 17 expected warm TLS sessions, the cap of 71, and zero application statements. It contains neither an enqueue receipt nor a workload ledger. The post-arm audit saw only the transient administrator connection. [Original controller release](evidence/first-setup-failed/controller-release.json) · [Original raw export](evidence/first-setup-failed/blocks/01-slow.1795493761849944000/warmup-mesh/warmup-mesh/raw.tar.gz)

The controller's adapter overlay repairs the SQL at its source. The broker calls `CURRENT_DATABASE()` on the arm's database. The post-arm admin connection substitutes the designated workload database for the complete function call, so it still audits that database while connected to `postgres`. Offline comparison verifies that those two lines are the only adapter changes. The original files, failure archive, binaries, PostgreSQL cap, launcher, and scorer remain sealed. [Exact overlay](sql-audit-retry/adapter.py)

An untimed PostgreSQL 17.11 regression passed using the actual statement extracted from the failure archive. It reproduced `42703` with the original bytes. The repaired broker and admin paths both returned the intended statement counts for the runner and application roles. The check also planted statements in another database and under the administrator role and verified their exclusion. The regression used only a disposable database query fixture and ran no queue workload. [Regression receipt](evidence/sql-audit-untimed.json)

The sole controller accepted separate SETUP and TIMING seals for the same retry. One complete slow arm from each of the four engines must qualify before the eight balanced timing blocks can start. The failed attempt stays separate from retry results. [SETUP seal](request-sql-retry-setup.json) · [TIMING seal](request-sql-retry-timing.json)

## Replay

From the repository root, replay the archived evidence without starting PostgreSQL.

```sh
packet=tools/durable-jobs-bench/results/2026-11-24-ordinary-slow-equal-topology-cap71
(cd "$packet" && shasum -a 256 -c SHA256SUMS.sql-audit-retry)
(cd "$packet/evidence/first-setup-failed" && shasum -a 256 -c SHA256SUMS)
python3 -B "$packet/replay-sql-audit.py"
```

Add `--pg-bin /opt/homebrew/opt/postgresql@17/bin` to reproduce the untimed SQL regression using PostgreSQL 17.11.
