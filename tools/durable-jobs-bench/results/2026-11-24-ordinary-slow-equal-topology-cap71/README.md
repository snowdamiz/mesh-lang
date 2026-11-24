# Slow 2×32 sensitivity at PostgreSQL cap 71

Cap 68 cannot accommodate the retained equal-topology peers. In five native equal-dense blocks, Apalis and Oban each held 69 connections in every block. River reached 70. In the separate cap-68 slow pilot, River, Oban, and Apalis each reached 64 overlapping committed holds at stock 1×64 and used a sampled peak of 67 connections. The alternate Mesh 2×32 arms also reached 64 overlapping holds, with 17 connections. Both prior series retain their original caps and scores. [23 sealed native witnesses](evidence/cap-proof.json)

## Physical budget

| Engine at 2×32 | In-runner app capacity | In-runner queue and listener headroom | Detached listeners | Shared producer and observer | Full allocation |
| --- | ---: | ---: | ---: | ---: | ---: |
| True-bulk V1 Mesh | 8 warm app sockets | 8 | 0 | 1 | 17 |
| Oban | 64 held app sockets | 4 | 0 | 1 | 69 |
| Apalis | 64 held app sockets | 4 | 0 | 1 | 69 |
| River | 64 held app sockets | 4 | 2 | 1 | 71 |

The peer handlers check out a connection before the first application commit and return it after the second. A 100 ms hold retains the checkout between commits. Mesh sleeps outside the database checkout and preserves the slow pilot's eight warm app sockets.

- **Oban.** Each Ecto pool has 34 connections. The isolated notifier and peer use no separate PostgreSQL listener. The one-connection session provides both enqueue and observation. A fully warm deployment requires 69 clients.
- **Apalis.** SQLx eagerly opens 34 connections per runner. `PgListener::connect_with` acquires its listener from that pool. One other session provides enqueue and observation. Cap 68 cannot satisfy the two pool minima and the broker simultaneously, even before the batch.
- **River.** Each lazy pgx pool permits 34 connections. River pgx driver 0.51.2 calls `poolConn.Hijack()` on its listener. pgx transfers the socket to the listener and puddle removes it from pool accounting, so the pool can open a replacement while the listener remains connected. Two clients can therefore use 68 pooled connections and two detached listeners. A third one-connection pool provides enqueue and observation. Cap 70 would still trim the permitted stock allocation. The measured high-water mark of 70 gives an empirical lower bound, and 71 prices the intact deployment envelope.

The benchmark starts the disposable cluster and finishes admin work before the timed submit. There is no separate observer connection. Post-run SQL collection uses one transient admin session after all clients close. The validator accounts for every successful PostgreSQL client authorization and disconnection and checks that admin access stays outside the full measured interval. PostgreSQL and its statistics extension are pinned to the retained 17.11 binaries. [Source anchors and native connection receipts](evidence/cap-proof.json)

## Prospective comparison

Each platform runs two independent 32-slot processes on CPUs 0 and 1. The arm uses the original 512 × 16 KiB payload input, 100 ms holds, separately committed attempt and effect writes, verified TLS, fsync, full-page writes, and synchronous commit. Cap 71 applies to the server and the workload database. Each platform keeps its stock listener, lease, settlement, and application checkout behavior. A result may observe fewer than 64 simultaneous holds even though the deployment permits 64. The replay reports actual overlap per process and in aggregate.

Mesh, Oban, and Apalis reuse their exact retained native executables. River reuses the existing configuration-only Linux ELF. The untimed build proof reproduced the stock ELF byte for byte before replacing the slow profile's runner count and connection cap on one line. No binary or workload ran locally for this follow-up. [River build receipt](evidence/river-build.json) · [One-line source diff](evidence/river-profile.diff)

SETUP qualifies one complete slow arm per platform. TIMING requires the four sealed SETUP archives to replay under the same binary and driver pins. Eight planned blocks give each platform two runs in each order position. The first two blocks reverse their order. Each platform receives a full warmup and a fresh measured arm in every block. The score reports submit-to-terminal and enqueue-commit-to-terminal, with within-block differences and full ranges. It also records runner CPU, CPU on the pinned pair, actual process ownership, the connection census, SQL claim and settlement counts, lease or maintenance SQL, and application and terminal transaction IDs. [SETUP intake](request-setup.json) · [TIMING intake](request-timing.json)

The verified artifact is a separate cap-71 sensitivity. Fresh native correctness and timing require the sole controller's grants after the booked sequence. [Offline replay instructions](REPLAY.md)
