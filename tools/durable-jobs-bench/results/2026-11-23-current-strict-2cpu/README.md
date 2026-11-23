# Current and historical Mesh on the original two-CPU workload

The first rotated group on November 23, 2026 compared current Mesh at
`3828f20d`, the historical `3f69b150` release build, Oban, and River.
It used CPUs 0 and 1 for both workloads and PostgreSQL. The two blocks
reconciled the original payloads, connection limits, verified TLS, two
independent application commits, and terminal commit timestamps.
[Current AMD64 artifact](../2026-11-23-native-mesh-current/README.md),
[engine pins and plan](series.json)

| Profile | Current Mesh | Historical Mesh | Oban | River |
| --- | ---: | ---: | ---: | ---: |
| Dense no-op submit jobs/s | 172.47 | 172.00 | 583.55 | 954.04 |
| Dense no-op commit jobs/s | 189.92 | 189.73 | 650.27 | 1,023.06 |
| Short submit jobs/s | 207.44 | 207.18 | 277.16 | 565.26 |
| Short commit jobs/s | 232.68 | 235.43 | 312.75 | 610.17 |

The series stopped after the first complete rotation to prioritize changed
designs. The saved comparison is a live reference for that rotation.
It includes raw arm files and SHA-256 receipts. [Replay index](summary-partial.json)
