# Unfused SLOW 2×32 codec timing

Five matched orders passed every original gate. Mesh led stock River on both durable clocks in all five under both codecs. Under LZ4, Mesh led Oban on both clocks in four orders and trailed Apalis in all five. The full release exited 1 because one warmup in order 6 breached the CPU-steal limit. All 96 full archives and all 48 pilot archives are retained. [Pilot release](evidence/pilot/controller-release.json) · [Full release](evidence/full/controller-release.json) · [Independent archive replay and exact per-order arithmetic](full-archive-replay.json)

The equal deployment gives all four engines two 32-slot runners, 71 PostgreSQL sessions, the same isolated PostgreSQL 17.11 binary and 512 canonical 16 KiB jobs with 100 ms holds between independent application commits. Mesh uses unfused V1 cursor claims through `claim_batch_v1`. All successful physical receipts show 512 LZ4 rows in the LZ4 arms and 512 uncompressed rows in the PGLZ arms, including Apalis’s stored `bytea`. The fresh PGLZ bridge keeps the PG image and combines a topology change with the original cap change from 68 to 71.

## Five orders that passed the noise gate

Each result is a median of within-order differences and its observed range, in milliseconds. Submit starts at the producer submit boundary. Drain starts at durable enqueue commit. Both end at the last durable settlement.

| Peer | Codec | Mesh minus peer, submit | Mesh minus peer, drain | Mesh leads, submit · drain |
| --- | --- | ---: | ---: | ---: |
| River | PGLZ | -202.0 [-287.6, -3.4] | -222.3 [-283.7, -42.1] | 5/5 · 5/5 |
| River | LZ4 | -186.9 [-206.9, -28.7] | -217.2 [-261.3, -58.5] | 5/5 · 5/5 |
| Oban | PGLZ | -169.8 [-306.8, -118.4] | -174.4 [-312.3, -104.1] | 5/5 · 5/5 |
| Oban | LZ4 | -137.5 [-202.1, +42.7] | -164.5 [-191.1, +10.5] | 4/5 · 4/5 |
| Apalis | PGLZ | +350.7 [+239.9, +418.6] | +167.2 [+73.3, +252.6] | 0/5 · 0/5 |
| Apalis | LZ4 | +330.0 [+309.7, +394.1] | +130.3 [+122.1, +221.0] | 0/5 · 0/5 |

The per-order change in Mesh’s submit difference against River after switching both sides to LZ4 was −4.6 ms median [−183.5, +251.5]. Against Oban it rose +55.8 ms [+27.0, +161.0], narrowing Mesh’s submit lead in all five orders. The drain change was −39.0 ms [−155.6, +207.6] against River and +68.1 ms [−60.4, +142.5] against Oban.

| Engine | PGLZ minus LZ4 enqueue | PGLZ minus LZ4 submit | PGLZ minus LZ4 drain |
| --- | ---: | ---: | ---: |
| Mesh | +140.7 [+133.5, +207.2] | +166.4 [+105.0, +252.3] | -13.3 [-33.0, +107.6] |
| River | +179.2 [+164.3, +183.1] | +247.1 [+10.3, +356.5] | +64.0 [-168.9, +174.7] |
| Oban | +164.3 [+114.7, +233.5] | +243.2 [+198.8, +308.1] | +101.8 [-34.7, +156.7] |
| Apalis | +167.0 [+161.2, +190.5] | +176.4 [+96.7, +207.8] | -3.3 [-64.5, +42.1] |

Positive values in the codec table favor LZ4. All four engines saved enqueue and full-submit time in every eligible order. Mesh’s LZ4 drain effect changed sign across orders.

| Engine | PGLZ 1×64 minus 2×32 submit | PGLZ 1×64 minus 2×32 drain |
| --- | ---: | ---: |
| Mesh | +137.1 [+110.8, +205.4] | +137.5 [+110.4, +219.6] |
| River | +0.4 [-140.5, +236.4] | +1.5 [-136.0, +266.1] |
| Oban | +81.7 [+17.4, +138.7] | +49.9 [+23.2, +202.2] |
| Apalis | -21.4 [-48.9, +40.5] | +28.6 [-54.0, +33.1] |

Positive values in the bridge table favor 2×32 cap 71. The River bridge spans both signs widely. The original 1×64 LZ4 series is a separate set of matched orders.

## Six orders for description

The sixth order retained all twelve measured arms. The 1×64 Apalis warmup rejected that rotation under the frozen gate. The values below show what all six orders recorded. [All six orders and exact codec, peer, bridge and rank contrasts](full-archive-replay.json)

| Peer | Codec | Mesh minus peer, submit | Mesh minus peer, drain | Mesh leads, submit · drain |
| --- | --- | ---: | ---: | ---: |
| River | PGLZ | -138.6 [-287.6, +237.7] | -152.3 [-283.7, +203.0] | 5/6 · 5/6 |
| River | LZ4 | -183.1 [-206.9, -28.7] | -215.0 [-261.3, -58.5] | 6/6 · 6/6 |
| Oban | PGLZ | -154.0 [-306.8, -39.4] | -149.3 [-312.3, -17.0] | 6/6 · 6/6 |
| Oban | LZ4 | -150.8 [-202.1, +42.7] | -160.1 [-191.1, +10.5] | 5/6 · 5/6 |
| Apalis | PGLZ | +376.6 [+239.9, +542.0] | +204.2 [+73.3, +414.8] | 0/6 · 0/6 |
| Apalis | LZ4 | +331.6 [+309.7, +394.1] | +148.3 [+122.1, +221.0] | 0/6 · 0/6 |

| Order | Mesh minus River PGLZ submit | PGLZ drain | Mesh minus River LZ4 submit | LZ4 drain |
| --- | ---: | ---: | ---: | ---: |
| 1 | -3.4 | -42.1 | -186.9 | -197.7 |
| 2 | -287.6 | -283.7 | -206.9 | -245.4 |
| 3 | -280.2 | -266.1 | -28.7 | -58.5 |
| 4 | -75.1 | -82.2 | -179.3 | -217.2 |
| 5 | -202.0 | -222.3 | -206.6 | -261.3 |
| 6 | +237.7 | +203.0 | -165.8 | -212.9 |

## Fresh first-write ranks and lifecycle

These are medians from the five eligible measured orders. Ranks sort each job’s durable first application commit. The hold count averages concurrent committed holds over each arm’s rank 64 to 448 interval, then takes the median across orders.

| Engine | Codec | Commit to rank 64, ms | Commit to rank 448, ms | Active committed holds |
| --- | --- | ---: | ---: | ---: |
| Mesh | PGLZ | 80.6 | 1007.6 | 46.1 |
| Mesh | LZ4 | 90.6 | 1007.2 | 46.5 |
| River | PGLZ | 318.9 | 1054.6 | 55.4 |
| River | LZ4 | 309.0 | 1035.2 | 54.3 |
| Oban | PGLZ | 260.2 | 1255.9 | 45.1 |
| Oban | LZ4 | 190.2 | 1135.2 | 44.9 |
| Apalis | PGLZ | 96.0 | 868.8 | 52.6 |
| Apalis | LZ4 | 91.8 | 867.5 | 52.7 |

Under LZ4, Mesh reached rank 64 before River in all five eligible orders by a median 224.3 ms [215.8, 235.7]. River covered rank 64 to 448 faster in all five by 163.7 ms [119.1, 286.9]. River’s last-effect-to-terminal interval was longer by 205.3 ms [125.2, 234.0]. The raw replay keeps these intervals and rank 64 to the final first write for every measured arm. It also preserves the seven idle TLS connections and stock River’s measured sampled peak of 69 in the eligible arms.

Each arm starts a fresh cluster and stock runner processes. The table uses monotonic setup and adapter stamps and the wall-clock difference from last durable settlement to the worker’s exit stamp. PGLZ precedes LZ4 in each cell. River readiness is the observed stock ready marker after Go dispatch, including migration, followed by about three seconds before submit.

| Engine | PG start to ready, ms | Runner dispatch to READY observation, ms | Adapter elapsed, ms | Terminal to worker exit, ms |
| --- | ---: | ---: | ---: | ---: |
| Mesh | 1375.0 / 1550.5 | 129.6 / 126.2 | 10380.0 / 13506.6 | 3307.8 / 6404.9 |
| River | 1381.6 / 1304.3 | 144.0 / 143.6 | 7698.1 / 7359.5 | 704.7 / 665.5 |
| Oban | 1449.8 / 1259.2 | 1295.5 / 1279.9 | 12390.3 / 11907.6 | 1121.0 / 1035.9 |
| Apalis | 1526.9 / 1363.1 | 314.0 / 320.1 | 7854.8 / 7727.6 | 822.6 / 853.9 |

The maximum submit-window steal among the five eligible rotations was 0.248%. Their maximum exit-sample steal was 0.662%, on measured order 3 Apalis LZ4. The longest eligible terminal-to-worker-exit delay was 6.533 s, on a Mesh LZ4 warmup in order 1. The monotonic recorder envelope and the actual exit intervals remain in the archive replay.

## Rejected warmup and custody

The order 6 1×64 Apalis PGLZ worker exited zero. Its scored window starts at submit epoch `1795635057004498` µs and ends at durable settlement epoch `1795635058474926` µs. The original scorer uses the last sample at or before submit and the first sample at or after settlement. In the raw `host-samples.jsonl` those are zero-based rows 24 and 32, epoch `1795635056843458` and `1795635058694340` µs. The worker captures the first eight fields of each CPU’s `/proc/stat` counters. The scorer uses the eighth field for steal and sums the deltas of all eight fields across both pinned CPUs. [Raw arm](evidence/full/06/warmup-one-apalis-pglz/warmup-rust/raw.tar.gz) · [Recorder source](../../../worker.py#L68) · [Comparison gate](../../../run.py#L181)

| CPU | Eight field deltas | Steal ticks | Total ticks |
| --- | --- | ---: | ---: |
| 0 | `44, 0, 29, 107, 1, 0, 3, 4` | 4 | 188 |
| 1 | `53, 0, 35, 92, 2, 0, 2, 0` | 0 | 184 |

That is **4/372 × 100 = 1.075268817204301%**. The original gate allows at most 1% and displays the rejection as 1.08%. Whole-recorder steal was 0.259%; the segment from the first sample at or after settlement to the last recorder sample registered zero additional steal ticks. The arm’s worker exit came 802.980 ms after the final durable settlement. Raw SHA-256 `4a54db0860083993645c16a1c8607182c9f4031174252e28ebaf694def9c3072`. The full request pins the recorder `16d1e01d19ee7b325908f07a69c6cdc772caf7d034ad79a6583494d2e107993b` and scorer `d3a49ae11628b723c5fa66f3517d11b3ec630053a10536df64c43a2c5f5eedd8`. [Full request](request-full.json) · [Full recorded replay](evidence/full/replay.json)

The full controller release at 19:33:54.027532 UTC records exit 1 and SHA-256 `75ef717b4e941fd19442b684c12eff25ddc6c8618ba2ff8b581682b5e21b4aaa`. The ancillary `replay-full` command returned exit 1 with “full canonical release or custody missing” because its release helper requires exit zero. The archived canonical release and all 96 hashes were independently verified. The earlier admission guard failed before the grant when it saw a receipt-only commit. The controller compared the sealed source blobs and used the existing reservation for the single full dispatch. [Ancillary replay exit](evidence/full/release-replay.log) · [Pre-grant guard log](evidence/full/pre-grant-commit-guard.txt)

The exit-1 original proof, its four reused bridge cells, and the exit-zero eight-cell amendment remain in their original archives. The five-order table is a subset selected by the frozen per-order gate. The complete six-order qualified viewer entry remains withheld. [Original proof](../evidence/proof/controller-release.json) · [Amended proof](evidence/proof/controller-release.json) · [Independent proof arithmetic](proof-archive-replay.json)
