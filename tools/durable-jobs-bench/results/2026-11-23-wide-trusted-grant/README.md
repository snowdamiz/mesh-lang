# Dense ordinary Mesh with up to 32 jobs per claim

Keep a maximum of 16 jobs per claim for the planned two-runner Linode comparison. On the local macOS ARM64 host, width 32 beat both same-executable width-16 controls on post-commit drain in one of four rotations and on full-submit throughput in two. Runner CPU savings stayed within the difference between duplicate controls in every rotation. The recorded one-minute load ranged from 33.8 to 126.6. This screen leaves the native effect of increasing grant size unresolved. [Four rotated groups and paired changes](result.json) · [Two-runner Linode comparison](../2026-11-23-dense-matched-topology/README.md)

| Lane | Submit jobs/s | Commit jobs/s | Runner CPU ms/job | Claim calls per 1,024 jobs | Mean occupied slots |
|---|---:|---:|---:|---:|---:|
| Original V1 executable, width 16 | 242 | 919 | 1.198 | 205 | 38.5 |
| Rebuilt executable, width 16 A | 243 | 665 | 1.362 | 224.5 | 41.2 |
| Same rebuilt executable, width 32 | 281 | 569 | 1.263 | 167.5 | 36.1 |
| Same rebuilt executable, width 16 B | 467 | 1,164 | 1.066 | 172.5 | 38.3 |

The table gives medians across groups. Relative to the mean of the two controls within each group, the paired median changes at width 32 were −6.1% for submit throughput, −26.5% for drain throughput and −0.8% for runner CPU. Each group rotated the additional original V1 executable separately. The reducer counts every job, verifies each job’s two independent application XIDs and terminal XID, and checks the warmed sessions, runner identities, workload digest and installed SQL definition.

## What ran

The two runners shared 64 slots and 32 warmed application connections. Both used two scheduler workers. The database was PostgreSQL 17.11 over verified TLS. The client used the original frozen compiler, application sources and native archive. The diagnostic runtime could reserve and decode 32 grants. After migration into a fresh database the width-32 arm modified one upper bound in the stock claim function. The stock SQL budgets for individual contexts, total reply bytes and oversized markers applied throughout. This diagnostic used ordinary trusted handlers, with one terminal transaction for each job. [Build pins](build.json) · [Application inputs](manifest.json) · [Comparison driver](compare.py)

In a separate logged run, 21 grant replies contained more than 16 jobs and the maximum was 32. Counts from PostgreSQL summed to 1,024. The proof and all 16 timing arms completed, accounting for 17,408 jobs. The host load and the large spread between identical controls limit timing inference. The first three build attempts stopped on the Rust toolchain path, initialization of the larger fixed-size Rust array and a rebuilt shared compiler with a different digest. The successful link used the recovered byte-identical compiler. [Server count lines](proof-counts.log.gz) · [Build attempts](build-attempts.tar.gz)

## Replay

From the checkout root run `python3 -B tools/durable-jobs-bench/results/2026-11-23-wide-trusted-grant/reduce.py`. This reads both compressed sets of original arm ledgers, runner logs and acceptance records, checks the build manifest and reproduces the published JSON. The separate server-log capture only contributes to the proof. [Measured arms](measured.tar.gz) · [Proof arm](proof.tar.gz)
