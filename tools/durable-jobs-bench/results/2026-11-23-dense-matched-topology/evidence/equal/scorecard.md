| Deployment | Submit jobs/s | Drain jobs/s | Submit wins by Mesh | Drain wins by Mesh |
| --- | ---: | ---: | ---: | ---: |
| Mesh V1 2 × 32 | 545.94 [535.53–560.41] | 766.91 [749.00–777.94] | — | — |
| Mesh V1 8 × 8 | 359.43 [327.95–361.43] | 446.38 [410.43–450.39] | 5/5 | 5/5 |
| Oban 2 × 32 | 780.50 [728.81–796.75] | 880.61 [818.54–893.17] | 0/5 | 0/5 |
| River 2 × 32 | 1002.35 [1001.09–1119.55] | 1091.82 [1078.18–1234.63] | 0/5 | 0/5 |
| Apalis 2 × 32 | 1352.34 [1228.31–1399.11] | 1425.24 [1289.57–1471.71] | 0/5 | 0/5 |

| Deployment | Median runner CPU (ms/job) | Physical conn. peak [range] | PG pool cap (runner roles) | Warm app | Used app [range] | Terminal transactions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Mesh V1 2 × 32 | 0.641 | 41 [41–41] | 8 queue + 32 app | 32 | 4–4 | 1024 |
| Mesh V1 8 × 8 | 1.620 | 65 [65–65] | 32 queue + 32 app | 32 | 16–16 | 1024 |
| Oban 2 × 32 | 3.002 | 69 [69–69] | 68 | shared | — | 1024 |
| River 2 × 32 | 0.429 | 68 [67–70] | 68 | shared | — | 12 |
| Apalis 2 × 32 | 0.353 | 69 [69–69] | 68 | shared | — | 58 |
