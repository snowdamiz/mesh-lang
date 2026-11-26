# Queue work in the direct-pool pilot

Direct binding plus four scheduler workers per runner raised runner lifetime CPU even as the runners issued fewer SQL statements. Counts come from the 12 measured Linode arms in the two opposed rotations at commit `f9002c6ce`. Each arm settled 1,024 jobs with 2,048 independent application transactions.

| Measured quantity | Block 1, service2 A / B | Block 1, direct4 | Block 2, service2 A / B | Block 2, direct4 |
|---|---:|---:|---:|---:|
| Claim calls | 166 / 168 | 115 | 168 / 167 | 110 |
| Runner SQL calls | 698 / 701 | 521 | 695 / 692 | 507 |
| Terminal transactions | 165 / 166 | 152 | 163 / 162 | 153 |
| Lifetime runner CPU, ms | 566.9 / 542.1 | 590.0 | 525.2 / 537.8 | 601.5 |
| Application backends with a whole CPU tick | 4 / 4 | 8 | 4 / 4 | 8 |

All arms recorded 136 SQL calls during the idle window. Direct4 saved at least 177 runner statements in each rotation. The CPU excess over the higher control was 23.2 and 63.8 ms. A larger number of runner SQL calls cannot explain that excess. The captures leave open the cost per call, scheduler work, and other client-side work.

We also checked the middle 512 second-write commits against [the application XID overlap bound](INFLIGHT-XIDS.md). The direct4 lower bound reached one in each block. Eight backends accumulated CPU over each run, but the XID evidence cannot establish that all eight processed assigned transactions simultaneously. The same bound reached as high as 21 in the separately measured prewarmed River middle windows.

To reproduce the table and overlap bounds, point the script at the `evidence/native` directory from the pool-supply archive in `f9002c6ce`. It verifies the released score digest and every measured archive.

```sh
python3 -B pool_supply_work.py "$native_evidence" | cmp - pool-supply-work.json
```

## CPU available for a cooperative application write

Direct binding with two scheduler workers per runner finished below the larger service2 duplicate's lifetime CPU in six silent controls. The tightest margin was **1.058 CPU-ms over 2,048 application writes**, or **0.517 µs per call**. Changing both writes to post an asynchronous request on every call would have to recover almost all of its cost elsewhere in that block.

| Silent control pair | Net CPU available forward / reverse (ms) | Per application write forward / reverse (µs) |
|---|---:|---:|
| Pool supply | 55.684 / 23.939 | 27.189 / 11.689 |
| Idle spin | 1.058 / 20.279 | 0.517 / 9.902 |
| Event park | 25.309 / 13.629 | 12.358 / 6.655 |

Each arm issued 1,024 first writes and 1,024 second writes, all on jobs with zero configured hold. Between effect ranks 256 and 768, direct2 averaged 1.90–2.04 jobs between their first and second commits. Direct4 averaged 3.25–3.54 on the same image. Warm River averaged 9.02–11.14 in its separate six-block release. These intervals include the first acknowledgement, application work and the second write. The ledger cannot assign the interval to the network or establish how many socket waits can release a worker. [Warm River occupancy](WARM-RIVER.md)

The earlier shared-readiness prototype at `e5e6da50c` specialized plain TCP. TLS took its connection-worker path. Its direct-socket throughput result does not measure a cooperative TLS reply on this workload. The native discriminator is whether a second admitted job makes durable progress while a first reply is held, with the original scheduler worker count, pool size, and independent commit acknowledgements. The CPU figures above describe net historical margins. Time saved through earlier completion could offset new runtime CPU.

Run `python3 -B direct2_wait_budget.py EXPOSURE_DIR` from this directory with the released event-park continuation. It checks all 24 arm archives, including process identities, per-statement row and call counts, lifetime CPU, and durable write ranks.

## How much of the prepared-statement saving belongs to the client?

The earlier isolated PostgreSQL trial recorded client and backend CPU separately. Taking the lower-CPU stock duplicate for each counter in each of six rotations, then subtracting named-statement CPU, gives a median **122.028 CPU-ms saved on the backend** and **6.055 CPU-ms added on the client** per 1,024 write pairs. The combined counter has its own duplicate comparison and yielded a median 115.973 CPU-ms saving. Backend savings can free time on the two shared cores. They do not directly pay the runner CPU excess.

The libpq trial sent a health query before each of the 2,048 writes and prepared each named statement before the CPU and wall brackets. In all six silent Direct2 and Direct4 product controls, the application role recorded only **4 and 8 health queries** respectively over those same 2,048 writes. The isolated trial saved 58.269 ms wall against a 75 ms floor and exceeded duplicate spread in three of six orders. A compiled literal-SQL optimization would need to price its own client work, first use, and actual checkout behavior in the application. [Native component replay](../2026-11-24-ordinary-app-plan-reuse/evidence/recovery-audit/README.md)
