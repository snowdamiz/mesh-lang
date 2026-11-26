# Handler progress against prewarmed River

Prewarmed River won all six matched 2×32 dense rotations on both durable clocks. Its paired median lead over Mesh was 272.638 ms from submit and 158.896 ms from enqueue commit. Warming the same River executable before submit saved a paired median 241.358 ms through settlement. Mesh started with 41 physical sessions, including 32 application sessions. Warm River started with 67. Each arm ran on two CPUs shared with PostgreSQL. [Released comparison](../2026-11-25-dense-fixed8-river-readiness/evidence/native/replay-rest.json)

Between the 256th and 768th committed second writes, **both Mesh runners had at least one entered handler still awaiting its second commit for the entire interval in every rotation**. A lack of entered work on either runner accounts for zero elapsed time in this window. Dispatch overhead can still compete with application work on the same CPUs.

| Rotation | Middle 512 writes, Mesh / warm River (ms) | Jobs between entry and first commit, Mesh / warm River | Jobs between first and second commit, Mesh / warm River |
|---|---:|---:|---:|
| 1 | 385.7 / 223.3 | 15.3 / 2.3 | 1.88 / 9.02 |
| 2 | 379.9 / 213.3 | 11.1 / 2.5 | 1.91 / 10.17 |
| 3 | 363.4 / 221.5 | 15.9 / 3.0 | 1.90 / 9.85 |
| 4 | 409.4 / 206.0 | 16.1 / 2.3 | 1.86 / 10.76 |
| 5 | 726.3 / 209.4 | 20.9 / 2.5 | 1.88 / 10.99 |
| 6 | 365.4 / 227.3 | 15.4 / 3.1 | 1.84 / 11.14 |

The counts are time-weighted concurrent jobs. Replay clips each job's recorded stage to the same rank window in its own run, sums those microseconds, and divides by the window length. Mesh records entry before looking up its application pool. River records entry before acquiring a connection. Every job commits two independent writes. Handler scheduling, connection acquisition, SQL execution, and durability can all contribute. These timestamps cannot assign CPU or wait time to any one of them. Rotation 5 is retained in full.

The results motivate testing two changes together on the fast grant and settlement configuration. Passing the application pool directly to a handler removes its service call before the first write. Increasing synchronous worker supply tests how many handlers can make progress through the second write. Earlier, on separate slower configurations, direct binding saved 34–70 ms of drain against the faster duplicate, and four workers saved 13 and 35 ms. Both earlier screens missed their advancement rules. A fresh comparison needs each component and their combination. [Direct binding](../2026-11-24-trusted-handler-app-pool/timing-clean-identity-retry/evidence/summary.json) · [Worker supply](../2026-11-24-dense-hot-worker-supply/NATIVE-RESULTS.md)

From this directory run `python3 -B warm_river_occupancy.py | cmp - warm-river-occupancy.json`. The replay binds all 24 measured archive and ledger hashes, checks job ordering and distinct application XIDs, and reproduces the published middle-512 clocks for Mesh and all three River controls.
