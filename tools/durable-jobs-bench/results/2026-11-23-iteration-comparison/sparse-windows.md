# Time between sparse submissions

Each arm’s full duration equals its 24 individual job response times plus the 23 intervals between a settlement and the next submit. Those intervals include 23 prescribed 250 ms pauses, totaling 5.75 seconds. The tables give the median of each measure across matched Linode rotations. Column medians can come from different rotations.

## Selected public build

5 matched rotations.

| Engine | Full arm (s) | Sum of 24 response times (s) | Sum of 23 intervals (s) | Median second application commit to settlement (ms) |
|---|---:|---:|---:|---:|
| Public trusted Mesh | 9.018 | 0.261 | 8.755 | 1.044 |
| Oban | 11.789 | 4.367 | 7.422 | 0.741 |
| River | 11.519 | 4.517 | 7.003 | 189.196 |
| Apalis | 9.680 | 0.238 | 9.443 | 1.068 |

## Original runners

5 matched rotations.

| Engine | Full arm (s) | Sum of 24 response times (s) | Sum of 23 intervals (s) | Median second application commit to settlement (ms) |
|---|---:|---:|---:|---:|
| Trusted Mesh V1 | 9.038 | 0.325 | 8.719 | 1.117 |
| Protected Mesh | 9.100 | 0.531 | 8.568 | 3.142 |
| Oban | 11.790 | 4.223 | 7.568 | 0.747 |
| River | 11.520 | 4.507 | 7.013 | 189.194 |
| Apalis | 9.643 | 0.229 | 9.412 | 1.075 |

[Per-job response percentiles](sparse-latency.md) · [Archive digests and whole-arm rates](scores.json)
