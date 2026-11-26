# Short16 River steady-service sensitivity

The five frozen exit-confirmed bulk D2 rotations contain a launch-readiness asymmetry. Stock River grew from four idle physical connections to a measured peak of 18 in every rotation. Trusted Mesh R2, checked and hot, held 13. Stock Oban and Apalis each recorded 19 idle and 19 during load. This replay rechecks all 30 scored arms, their raw hashes, both durable clocks, and the distinct application transaction IDs. The original series keeps its published stock labels and results. [Stock replay](evidence/stock-replay.json) · [Original native results](../2026-11-24-exit-confirmed-bulk-d2/native-results.md)

| Rotation | River commit → 16th second write (ms) | River next 112 writes (ms) | River after last write → settlement (ms) | Mesh hot commit → 16th second write (ms) | Mesh hot next 112 writes (ms) | Mesh hot after last write → settlement (ms) |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 74.449 | 74.999 | 10.947 | 18.694 | 165.105 | 1.599 |
| 2 | 89.419 | 79.472 | 40.042 | 21.733 | 145.529 | 0.704 |
| 3 | 76.111 | 85.321 | 94.802 | 20.396 | 137.132 | 4.339 |
| 4 | 84.827 | 78.972 | 49.774 | 20.621 | 144.322 | 0.876 |
| 5 | 73.902 | 80.217 | 11.024 | 21.478 | 192.807 | 2.235 |

Mesh hot led stock River in rotations 2, 3 and 4 on both clocks. The median paired throughput difference was +10.67% from submit, with a range of −32.08% to +35.90%, and +24.39% from enqueue commit, with a range of −23.73% to +58.30%. River's early window was longer in all five rotations, its remaining write window was shorter in all five, and its final settlement tail ranged from 10.95 to 94.80 ms. These are observations under the frozen launch policies. The matched experiment below isolates an explicit change to River pool readiness. [Per-rotation clocks and connection samples](evidence/stock-replay.json)

## Experiment

Use a fresh PostgreSQL 17.11 cluster per arm on the existing Linode, with verified loopback TLS, CPUs 0 and 1, 16 slots, and a server and database cap of 20 client sessions. Each arm submits the same 128 jobs in one transaction. Each job commits an attempt and effect independently. Score from before batch read and from the durable enqueue commit to the last durable settlement. Retain the checked and hot R2 writer-off runners, stock Oban and stock Apalis from the original series, and the exact frozen River ELF as a duplicate cold control. The new River ELF runs twice with the same wrapper, standard River client, shared pool and ordinary handler. Only `RIVER_BENCH_POOL_READINESS=warm` explicitly fills the pool. The `cold` arm adds the same startup observations and otherwise uses the lazy pool. The Go 1.26.4 build reproduced the original River ELF byte for byte before building the new one. [Build and dependency pins](evidence/build.json) · [Controller packet](controller-packet.json)

The new warm runner acquires 17 distinct connections from its stock 18-slot pgxpool before starting the River client. It holds them only during fill, releases every checkout before `Client.Start`, and bounds checkout with a 10 s context. This keeps the pool available for River's driver initialization, listener and lifecycle. The pinned notifier initially connects during startup and the pgx driver hijacks a pooled connection for LISTEN. This leaves 16 warmed pooled connections, one detached listener, and the existing one-connection producer/observer, for 18 physical sessions. The stock possible maximum is 18 in the shared pool plus those two other connections. PostgreSQL enforces the physical ceiling of 20 even if the listener reconnects. SETUP verifies the real same-role TLS backend IDs and the matching detached listener, checks the empty job, attempt, effect and product tables at all four points, and requires the same 18 physical backend identities across the three ready samples through submit. The fill only acquires and releases connections. Normal River startup can still perform queue metadata maintenance. [River client lifecycle](https://github.com/riverqueue/river/blob/v0.51.2/client.go) · [Notifier](https://github.com/riverqueue/river/blob/v0.51.2/internal/notifier/notifier.go) · [pgx LISTEN driver](https://github.com/riverqueue/river/blob/v0.51.2/riverdriver/riverpgxv5/river_pgx_v5_driver.go)

The startup receipt prices producer connection setup and health checks, runner launch through readiness, the explicit pool fill, and the common three-second idle window separately from the two service clocks. It carries actual Go producer and runner CPU counters, per-client PostgreSQL backend CPU counters, threads and RSS. PostgreSQL CPU covers the visible service client processes. RSS includes shared pages. These receipts exclude disposable cluster boot and schema provisioning from the runner startup window. CPU precision is the host's procfs tick rate, and the runner readiness poll is at 20 ms.

Three mirror-image order pairs put all seven lanes on both sides of the rotation midpoint or in its center. SETUP uses three fresh correctness arms and never adds their measurements to the six timed blocks. TIMING opens with the first mirrored pair. The four remaining blocks require a successful fenced pilot release and full replay of both pilot rotations. Report the paired warm-minus-cold effect, the candidate-cold versus original-stock control, and both R2 comparisons to each River arm. Retain per-rotation and distribution values. Split durable first-write, second-write and terminal clocks at transaction rank 16 with the 112-transaction remainder. Also report per-job intervals separately for the first 16 and last 112 second-write commits and the end-of-run settlement tail. [Scorer](score.py)

## Controller intake

From the repository root, the offline check rebuilds the result from archives and validates the staged ELF and inputs. The build can also be repeated separately with the pinned Go toolchain and a populated module cache.

```sh
r=tools/durable-jobs-bench/results/2026-11-24-short16-river-readiness
python3 -B "$r/controller.py" check
python3 -B "$r/build.py" target/short16-reproduced
```

Only the native matrix controller runs the grant commands. Replace `SEAL` with the SHA-256 of the exact queued packet. Run each command in its scheduled slot after earlier bookings. The runner keeps setup attempts, raw archives, logs and release receipts under `evidence/`.

```sh
python3 -B "$r/slots.py" check --seal SEAL
python3 -B "$r/slots.py" setup --seal SEAL
python3 -B "$r/slots.py" pilot --seal SEAL
python3 -B "$r/slots.py" rest --seal SEAL
```
