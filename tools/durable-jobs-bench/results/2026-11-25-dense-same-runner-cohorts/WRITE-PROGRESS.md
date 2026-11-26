# Dense 8×8 application progress against stock River

Across all five released rotations, the interval between Mesh handlers 256 and
768 entering was already 188–838 ms longer than River's. The corresponding
interval between durable effects was 191–947 ms longer. These are independently
ranked timestamps for each landmark. Different jobs can occupy a given rank.

| Rotation | Claim stamp | Handler entry | First write commit | Effect commit |
|---|---:|---:|---:|---:|
| 1 | +168 ms | +188 ms | +196 ms | +191 ms |
| 2 | +831 ms | +838 ms | +938 ms | +947 ms |
| 3 | +212 ms | +217 ms | +222 ms | +227 ms |
| 4 | +254 ms | +264 ms | +264 ms | +258 ms |
| 5 | +228 ms | +224 ms | +230 ms | +239 ms |

Each column gives the Mesh interval minus the River interval. A claim stamp
precedes the grant receipt. Mesh stamps are millisecond precision. Rotation 2
retains the 22 contention hints. The measurements cannot identify a specific
function or prove that a delay started at the claim. Finishing an application
write can advance a later grant through slot reuse.

River's pinned worker borrows its application connection once and runs two
autocommit INSERTs. Mesh uses two pool checkouts. Each Mesh arm recorded
2,048 INSERTs and only 16 health probes. The application receipts identify
transactions and the Mesh runner, but omit the PostgreSQL backend identity
for each write. The frequency of changing backend between writes is unknown.

On Mesh the median individual interval from first to second commit was
1.99–4.54 ms across rotations. River's was 5.74–6.23 ms. With all observed
first commits held fixed, even granting the entire second write zero time
would advance Mesh's last effect by just 0.43–2.37 ms. A material gain from
a shared checkout would need to advance *subsequent* jobs' first commits as
well. The released evidence gives little reason to implement a new pool API
for this benchmark. Direct application-pool binding instead affects the path
before the first write and has its own separate two-runner evidence.

Reproduce the ten archive checks and rank calculations from this directory
with `python3 -B write_progress.py | diff - write-progress.json`. For the
original formal decisions see the [released paired comparison](../2026-11-25-public-dense-late-capacity/peer-comparison/RESULTS.md).

## How much time accrued between successive durable effects

Each middle window contains 512 intervals between consecutive second-write commits. The table compares the sum of each lane's ten longest intervals. The last column removes all ten from each lane and compares the remaining 502 intervals. Times are milliseconds.

| Rotation | Mesh B middle | River middle | B longest ten | River longest ten | B excess over remaining 502 |
|---|---:|---:|---:|---:|---:|
| 1 | 507 | 316 | 55 | 56 | 192 |
| 2 | 1,238 | 291 | 286 | 46 | 707 |
| 3 | 526 | 298 | 55 | 50 | 222 |
| 4 | 538 | 280 | 55 | 47 | 250 |
| 5 | 534 | 296 | 54 | 52 | 237 |

Even in rotation 2, replacing Mesh's ten longest intervals with River's ten longest accounts for only 240 of the 947 ms difference. The two same-image OFF controls spent 60 and 55 ms in their longest ten that rotation. In every Mesh rotation, at least nine of the ten longest intervals contained a durable first write, and at least nine contained a terminal commit. That count uses timestamps strictly between the adjacent effects and excludes ties at the endpoints. The database continued making observable progress during these intervals.

A change that only shortens the largest pauses would leave most of the middle gap. An improvement in the ordinary rate of starting and completing jobs could affect hundreds of intervals. This arithmetic compares independently sorted intervals. It does not associate a particular slow interval with a matching job in another engine or estimate the downstream effect of changing a claim or settlement.

The [reducer](effect_gaps.py) verifies the 25 measured archive hashes for B, both same-image OFF controls, River and Oban. It checks 1,024 completed jobs and 2,048 distinct application transactions per arm. The [full reduction](effect-gaps.json) retains six cutoffs from one to 100 intervals.
