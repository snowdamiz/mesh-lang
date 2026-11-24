| Deployment | Submit jobs/s | Drain jobs/s | Submit wins by Mesh | Drain wins by Mesh |
| --- | ---: | ---: | ---: | ---: |
| Mesh V1 2 × 32 | 547.01 [542.04–555.16] | 751.82 [745.58–783.04] | — | — |
| Mesh V1 8 × 8 | 362.63 [341.45–368.82] | 452.45 [420.14–462.75] | 5/5 | 5/5 |
| Oban 8 × 8 | 633.50 [596.22–661.53] | 695.27 [663.74–726.79] | 0/5 | 5/5 |
| River 8 × 8 | 821.00 [820.26–876.01] | 895.20 [877.23–933.46] | 0/5 | 0/5 |
| Apalis 8 × 8 | 1113.61 [1098.44–1189.98] | 1161.49 [1142.31–1242.82] | 0/5 | 0/5 |

| Deployment | Median runner CPU (ms/job) | Physical conn. peak [range] | PG pool cap (runner roles) | Warm app | Used app [range] | Terminal transactions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Mesh V1 2 × 32 | 0.637 | 41 [41–41] | 8 queue + 32 app | 32 | 4–4 | 1024 |
| Mesh V1 8 × 8 | 1.615 | 65 [65–65] | 32 queue + 32 app | 32 | 14–16 | 1024 |
| Oban 8 × 8 | 9.852 | 81 [81–81] | 80 | shared | — | 1024 |
| River 8 × 8 | 0.764 | 80 [80–81] | 80 | shared | — | 36 |
| Apalis 8 × 8 | 0.603 | 81 [81–81] | 80 | shared | — | 181 |
