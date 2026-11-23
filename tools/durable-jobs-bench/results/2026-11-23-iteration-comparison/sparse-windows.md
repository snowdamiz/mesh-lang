# Time between sparse submissions

Each of the 5 rotations submitted 24 jobs individually and waited for each to settle. Each arm’s full duration equals its 24 job response times plus the 23 intervals between a settlement and the next submit. Those intervals include a prescribed 250 ms pause, or 5.75 seconds per arm. The table gives the median of each measure across rotations.

| Engine | Full arm (s) | Sum of 24 response times (s) | Sum of 23 intervals (s) | Median second application commit to settlement (ms) |
|---|---:|---:|---:|---:|
| Trusted Mesh V1 | 9.038 | 0.325 | 8.719 | 1.117 |
| Protected Mesh | 9.100 | 0.531 | 8.568 | 3.142 |
| Oban | 11.790 | 4.223 | 7.568 | 0.747 |
| River | 11.520 | 4.507 | 7.013 | 189.194 |
| Apalis | 9.643 | 0.229 | 9.412 | 1.075 |

Apalis finished individual jobs fastest at the median. Its longer intervals between submissions affected the whole-arm rate. River picked jobs up quickly and spent most of each response waiting for the terminal commit after its second application write. Independently reported column medians can come from different rotations. [Per-job response percentiles](sparse-latency.md) · [Archive digests and whole-arm rates](scores.json)
