# Set-based producer on ordinary V1 slow

Five matched native blocks completed on November 23, 2026. Each arm ran
512 slow jobs with identical handler input, storage policy, and two CPU
affinity. PostgreSQL 17.11 and the runners shared CPUs 0 and 1. The bulk
and scalar arms ran the same ordinary V1 ELF. Every arm reconciled both
independent application writes and the terminal ledger. The original V1,
strict protected Mesh, Oban, River, and Apalis were fresh control arms.
[Frozen inventory](series.json) · [Native wire and smoke qualification](../native-setup-3b09dc3d0e23/README.md)

| Lane | Jobs/s from submit, median [range] | Jobs/s from first commit | First submit to commit (ms) | Submit-to-settle p99 (ms) | Runner CPU (ms/job) | Pinned CPU (ms/job) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Bulk | 261.68 [256.07–265.09] | 356.65 | 520.96 | 1,953 | 0.76 | 4.41 |
| Same-ELF scalar | 250.39 [232.45–253.76] | 366.52 | 659.37 | 2,041 | 0.75 | 4.57 |
| Original ordinary V1 | 251.24 [246.69–254.82] | 365.48 | 644.75 | 2,034 | 0.75 | 4.59 |
| Protected Mesh | 164.77 [162.94–166.72] | 209.05 | 649.62 | 3,102 | 4.71 | 10.22 |
| Oban | 247.27 [243.66–256.38] | 331.48 | 493.76 | 2,069 | 3.96 | 4.63 |
| River | 288.66 [253.30–288.96] | 387.57 | 463.37 | 1,774 | 0.90 | 3.95 |
| Apalis | 355.45 [282.52–373.52] | 457.84 | 342.71 | 1,440 | 0.48 | 3.15 |

Bulk reduced the first-submit-to-commit interval by a paired median
of 133.90 ms versus scalar, with a range of 89.71–199.79 ms. It completed
ahead of scalar and original V1 in all five full-submit windows. It
finished ahead of Oban in four blocks, River in one, and Apalis in zero.
Median drain throughput was lower than same-image scalar. The CPU columns
cover the runner lifetime and pinned submit-to-settlement window
respectively. The 70 warmup and measured archives and the observed
setup-inclusive arm wall time are retained. [All measured windows and raw hashes](summary.json)
