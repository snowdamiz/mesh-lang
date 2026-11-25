# When dense application writes return to the Mesh handler

Passing the application pool directly to the handler brought the first application commit forward by a paired median 6.01 ms per job and shortened the full 1,024-job drain by 44.47 ms. Both improved in six of six unobserved Linode rotations against the faster matched service control for each metric. Submit-to-settlement improved in four, by a paired median 30.45 ms. Three rotations cleared duplicate-control variation on all three metrics. The frozen gate required five and a median drain gain of 62.4 ms. The pilot closed without adoption. [Official paired decision](../2026-11-24-trusted-handler-app-pool/timing-clean-identity-retry/evidence/timing/replay-b5d363d8f458.json)

The separate instrumented arms join PostgreSQL's commit record to the return of each ordinary INSERT in the compiled handler. For the first write the observed window begins when the pool lookup returns. For the second it begins when the first write returns. Both use the same two independent autocommits, checked application pools and two scheduler workers per runner. Six direct and six service arms yield 24,576 matched write returns. By summing per-call elapsed durations, **65–75% of the observed direct-handler write time falls after the commit record**. The fraction was 65–66% in five of the six direct arms. [Raw digests, counts and durations](native.json)

| Dense group | Median record to return (µs), direct | Post-record share of direct write time | Either runner in the overlap window | Both runners at once | Observer drain change vs direct (ms) |
|---:|---:|---:|---:|---:|---:|
| 1 | 929 | 65.6% | 54.1% | 14.0% | −4.9 |
| 2 | 792.5 | 65.5% | 47.2% | 10.6% | −19.6 |
| 3 | 1,280 | 74.6% | 68.8% | 23.5% | +391.4 |
| 4 | 934 | 65.8% | 54.6% | 14.7% | +49.3 |
| 5 | 856 | 65.1% | 50.1% | 11.8% | −4.2 |
| 6 | 946.5 | 65.7% | 53.2% | 13.0% | −9.4 |

The overlap columns cover elapsed wall time from the 103rd through the 922nd committed second write. A runner counts when two or more observed calls have reached their PostgreSQL commit record and have yet to return, and zero observed calls on the same runner occupy the preceding write interval. In the service arms, another handler was inside the pool lookup for 97.6–99.7% of the combined runner time satisfying those conditions. The logs give handler boundaries. They cannot establish whether a scheduler thread was asleep or runnable, or how much work another thread could have completed.

PostgreSQL can assign the commit timestamp before a synchronous WAL flush finishes. The remaining interval may also include PostgreSQL scheduling, delivery of ReadyForQuery, the driver's reply processing and scheduling of the application actor. The group-three observer adds 391 ms of drain relative to its unobserved direct counterpart. The other five observed arms retain a substantial return interval. These measurements apply to the scalar producer and checked pool used here. Testing the public hot pool still requires its own comparison.

To distinguish database completion from delivery back to Mesh, the next diagnostic needs a server timestamp **after the local synchronous flush** joined to the same application XID and caller return, plus matched observer overhead. The existing cluster-wide sync times cannot allocate a flush to an individual write. [Native WAL counters](../2026-11-24-dense-wal-cost/NATIVE-RESULTS.md)

## Offline replay

From the checkout root, read all 30 measured dense exports and compare the reduction with the archived result. The other 78 arms in the original series comprised the six short rotations, dense warmups and the frozen V1 dense reference.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-handler-commit-return
python3 -B "$d/replay.py" | cmp - "$d/native.json"
python3 -B "$d/test_overlap.py"
```
