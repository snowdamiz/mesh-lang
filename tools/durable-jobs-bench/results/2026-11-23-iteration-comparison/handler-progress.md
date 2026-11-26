# When handlers first enter and when the queue finishes

The clocks start at each arm’s own PostgreSQL enqueue commit. Handler-entry ranks count distinct jobs. A last-stage timestamp is the maximum across the batch, so the last entry, last write, and last settlement can belong to different jobs. All times are elapsed milliseconds. Each arm ran on CPUs 0 and 1.

## Short jobs with set-based enqueue and grouped successes

| Rotation | Engine | First entry | Half entered | Last entry | Second half of entries | Last first write | Last second write | Last settlement |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | Trusted Mesh | 5.5 | 87.0 | 168.9 | 81.9 | 175.2 | 178.1 | 179.3 |
| 1 | River | 2.9 | 100.3 | 145.6 | 45.3 | 145.9 | 147.7 | 169.7 |
| 2 | Trusted Mesh | 6.0 | 83.3 | 168.3 | 85.0 | 171.6 | 173.7 | 176.4 |
| 2 | River | 2.6 | 107.0 | 150.3 | 43.3 | 150.4 | 152.1 | 160.9 |
| 3 | Trusted Mesh | 5.5 | 90.1 | 171.6 | 81.5 | 176.2 | 178.5 | 179.9 |
| 3 | River | 3.4 | 136.2 | 181.9 | 45.7 | 182.1 | 185.2 | 208.1 |
| 4 | Trusted Mesh | 5.8 | 87.0 | 172.2 | 85.2 | 173.7 | 176.1 | 179.8 |
| 4 | River | 2.8 | 106.6 | 143.2 | 36.6 | 143.8 | 146.8 | 167.8 |
| 5 | Trusted Mesh | 5.7 | 81.9 | 163.9 | 82.0 | 166.8 | 168.9 | 171.7 |
| 5 | River | 3.2 | 135.1 | 184.2 | 49.1 | 184.5 | 186.6 | 213.4 |

### Mesh backlog when River finished

The following counts overlap. Each shows how many Mesh jobs still had the indicated step ahead of them when the peer reached its last settlement. The claim range allows for the millisecond resolution of the Mesh stamp and uses handler entry as its upper bound.

| Rotation | Peer finished, ms | Unclaimed | Awaiting entry | Awaiting first write | Awaiting second write | Awaiting settlement |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 169.7 | 0–0 | 0 | 3 | 9 | 13 |
| 2 | 160.9 | 0–0 | 9 | 12 | 15 | 15 |
| 3 | 208.1 | 0–0 | 0 | 0 | 0 | 0 |
| 4 | 167.8 | 1–1 | 2 | 4 | 14 | 16 |
| 5 | 213.4 | 0–0 | 0 | 0 | 0 | 0 |

[Matched short run ledgers](../2026-11-23-bulk-enqueue-v2-short/evidence/bulk-enqueue-v2-short)

## Large jobs with EXTERNAL arguments and 16 MiB work_mem

| Rotation | Engine | First entry | Half entered | Last entry | Second half of entries | Last first write | Last second write | Last settlement |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | Trusted Mesh | 88.5 | 315.5 | 558.4 | 242.9 | 563.9 | 574.5 | 580.6 |
| 1 | Apalis | 56.6 | 126.4 | 176.1 | 49.7 | 177.4 | 182.6 | 188.0 |
| 2 | Trusted Mesh | 96.6 | 313.7 | 582.4 | 268.7 | 589.2 | 593.8 | 604.6 |
| 2 | Apalis | 111.7 | 142.1 | 164.7 | 22.5 | 177.8 | 190.0 | 203.5 |
| 3 | Trusted Mesh | 91.8 | 325.9 | 588.8 | 262.8 | 593.8 | 603.7 | 609.1 |
| 3 | Apalis | 56.4 | 133.3 | 173.2 | 39.9 | 179.4 | 190.4 | 199.0 |
| 4 | Trusted Mesh | 87.0 | 342.9 | 609.4 | 266.5 | 615.6 | 622.7 | 628.1 |
| 4 | Apalis | 76.8 | 107.7 | 173.1 | 65.5 | 177.7 | 184.0 | 192.4 |
| 5 | Trusted Mesh | 73.9 | 282.7 | 521.3 | 238.6 | 528.8 | 531.6 | 540.0 |
| 5 | Apalis | 38.4 | 148.8 | 194.5 | 45.7 | 200.0 | 210.9 | 217.9 |

### Mesh backlog when Apalis finished

The following counts overlap. Each shows how many Mesh jobs still had the indicated step ahead of them when the peer reached its last settlement. The claim range allows for the millisecond resolution of the Mesh stamp and uses handler entry as its upper bound.

| Rotation | Peer finished, ms | Unclaimed | Awaiting entry | Awaiting first write | Awaiting second write | Awaiting settlement |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 188.0 | 48–48 | 51 | 54 | 56 | 56 |
| 2 | 203.5 | 46–47 | 48 | 48 | 48 | 54 |
| 3 | 199.0 | 40–40 | 48 | 48 | 52 | 56 |
| 4 | 192.4 | 46–46 | 48 | 51 | 55 | 56 |
| 5 | 217.9 | 40–40 | 48 | 48 | 48 | 48 |

[Matched large run ledgers](../2026-11-23-ordinary-large-workmem/evidence/ordinary-large-workmem-matched)

These are observed arrival and commit times. The backlog at a fixed cutoff can change when an earlier stage changes. [Four-engine scores](four-engine-scorecard.md) · [Replay](handler-progress.py)
