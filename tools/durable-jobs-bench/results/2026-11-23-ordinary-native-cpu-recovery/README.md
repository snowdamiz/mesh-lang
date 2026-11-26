# Native-02 dense CPU recovery

All 18 arms reconciled 1,024 terminal jobs apiece, with the two distinct durable handler commits per job and the frozen enqueue receipt. The controller released the original reservation at 23:10:20.338209 UTC with diagnostic exit 1. Its original role qualification remains in force. River alone passed the drain controls in both orders. [Controller receipt](evidence/native-02/controller/status.json) · [Original decisions](evidence/native-02/summary.json) · [Recovered scores and intervals](decisions.json)

Across the four uninstrumented arms for each engine, Mesh had a drain busy-CPU lower bound of **3.135 CPU-ms/job** and Oban had an upper bound of **2.900 CPU-ms/job**. The weakest Mesh bound is **8.1%** above the strongest Oban upper bound. River's highest upper bound was **2.244 CPU-ms/job**. The Mesh drain also had **1.450–1.885 CPU-ms/job** of minimum busy CPU outside the runner processes across the four controls, even charging their entire lifetime `wait4` CPU to the drain. This is evidence from two orders on one pinned boot. It quantifies the excess without locating it within PostgreSQL, the broker, or the remaining host work.

## All 18 arms

CPU values bound busy time on the two pinned workload cores. Elapsed is the common durable ledger interval. Drain starts at the last enqueue commit. Full starts at first submit. Both end at last terminal settlement. The replay also retains producer, acknowledgement-to-settlement, and 102nd-to-922nd settlement windows.

| Order | Lane | Arm | Drain ms | Drain CPU-ms/job | Full ms | Full CPU-ms/job |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| Forward | Mesh | Off before | 2210.29 | 3.135–4.317 | 2768.34 | 3.936–5.407 |
| Forward | Mesh | On | 2377.59 | 4.336–4.590 | 2955.58 | 5.098–5.322 |
| Forward | Mesh | Off after | 2322.26 | 3.506–4.536 | 2889.91 | 4.316–5.557 |
| Forward | Oban | Off before | 1410.78 | 1.807–2.755 | 1586.44 | 2.295–3.099 |
| Forward | Oban | On | 1708.63 | 3.203–3.337 | 1914.16 | 3.428–3.633 |
| Forward | Oban | Off after | 1453.29 | 1.855–2.838 | 1601.35 | 2.334–3.128 |
| Forward | River | Off before | 1091.42 | 0.820–2.132 | 1175.85 | 1.299–2.285 |
| Forward | River | On | 1124.47 | 1.699–1.914 | 1197.38 | 1.758–1.973 |
| Forward | River | Off after | 1148.75 | 1.201–2.244 | 1223.21 | 1.201–2.354 |
| Reverse | River | Off before | 1128.09 | 0.801–2.197 | 1208.83 | 1.289–2.256 |
| Reverse | River | On | 1112.37 | 1.680–1.904 | 1194.07 | 1.787–1.992 |
| Reverse | River | Off after | 1146.97 | 1.250–2.240 | 1222.75 | 1.250–2.295 |
| Reverse | Oban | Off before | 1484.61 | 1.807–2.900 | 1654.01 | 2.275–3.230 |
| Reverse | Oban | On | 1812.59 | 3.105–3.398 | 2033.21 | 3.398–3.662 |
| Reverse | Oban | Off after | 1437.97 | 1.836–2.809 | 1603.08 | 1.836–3.131 |
| Reverse | Mesh | Off before | 2304.56 | 3.135–4.501 | 2878.02 | 4.297–5.527 |
| Reverse | Mesh | On | 2427.51 | 4.580–4.741 | 3023.72 | 5.332–5.527 |
| Reverse | Mesh | Off after | 2244.30 | 3.174–4.383 | 2799.94 | 4.326–5.469 |

Clean arms use the original sparse host samples. Their serial read bounds place the /proc/stat read after the prior epoch stamp and before the current one. True-on arms independently bracket the original nested /proc/stat read timestamps with the existing clock, width, cadence, edge and tick rules, then intersect those bounds with the sparse bounds. Whole process-scan time has its own coverage decision. All windows use the verified CPU mask, clock rate, durability and artifact pins. The sparse central settlement windows overlap across Mesh and Oban. The all-control separation above applies to the full drain window.

## Observer cost and control decisions

| Lane | Order | Spare-CPU reporter during drain, CPU-ms | Drain elapsed shift vs off midpoint | Drain control |
| --- | --- | ---: | ---: | --- |
| Mesh | Forward | 431.6–444.7 | +4.91% | Pass |
| Mesh | Reverse | 447.2–460.7 | +6.73% | Fail |
| Oban | Forward | 1427.3–1483.3 | +19.31% | Fail |
| Oban | Reverse | 1467.0–1528.8 | +24.04% | Fail |
| River | Forward | 22.0–23.8 | +0.39% | Pass |
| River | Reverse | 22.1–23.8 | −2.21% | Pass |

The spare-CPU column uses the archived reporter thread CPU clock. The read follows the current frame end and precedes the next frame start, so the replay brackets that read independently. These are real recorder costs outside the two charged cores. The shifts measure the observed paired wall-time perturbation, which can include indirect contention. The original matched-control thresholds apply unchanged. Runner lifetime CPU passed all six controls. River's forward producer failed its spread and shift checks, so its both-order pass covers drain only. The Mesh forward control pass does not restore a role registry.

## Process evidence

**Mesh.** The readiness TLS census captured **23 Jobs + 32 application + 1 broker DB** forward and **26 + 32 + 1** in reverse. The collector retried the same rejected census in 305 frames in each order. The canonical observer sampled 56/59 at the first later idle point and 65 at the second. Every measured canonical connection count in both probes was 65. Its SQL counts only the three workload logins in the trial database. The sealed login caps of 32 Jobs, 32 application and one producer force the observed 65 to be **32 + 32 + 1 simultaneously connected** at those points. The producer login also executes harness observer queries through the persistent broker. These counts provide session concurrency at sample time. Individual CPU clocks and query concurrency within the interval are unidentifiable.

The two Mesh ledgers recorded seven and eight distinct runners handling jobs. The runner lifetime `wait4` CPU gives drain upper bounds of 1.667 and 1.708 CPU-ms/job. The probes therefore leave at least 2.669 and 2.872 CPU-ms/job outside runners. There are no runner, broker, postmaster or PostgreSQL process-clock rows to split that remainder. The application role executed 2,048 checkout probes and 1,024 of each durable insert per probe. Its summed PostgreSQL statement execution elapsed was 110.106 and 127.977 ms. The application backend CPU remains unidentifiable.

**Oban.** The sealed census has one producer/observer login and 80 Jobs backends. There are 179 and 191 complete process frames including the initial registry frames, plus one errored partial process dictionary per arm excluded from reduction. There are 41 and 44 complete frames wholly inside drain. Maximum interior scan duration is 71.76 and 74.91 ms, past the existing 25 ms qualification limit. The separately logged 92 roster comparisons failed in each order, first 146.46 and 194.44 ms after terminal settlement. The raw error does not identify which live PIDs or titles caused the discrepancy.

The following are PID-clock bounds from stable captured identities in the instrumented Oban probes. Each lower bound uses two reads wholly inside drain. Each upper bound encloses drain for those same PIDs. Idle or retired backends outside the registered set and runner descendants remain in the host residual. Full-role upper bounds from that residual are in the JSON. No reads are synthesized to fill missing process frames.

| Captured PIDs | Forward CPU-ms/job | Reverse CPU-ms/job |
| --- | ---: | ---: |
| Eight runner roots | 1.390–1.456 | 1.409–1.467 |
| Broker root | 0.013–0.033 | 0.015–0.037 |
| 80 Jobs backends | 1.559–1.593 | 1.495–1.566 |
| Producer/observer backend | 0.013–0.058 | 0.015–0.043 |
| Postmaster | 0.000–0.000 | 0.000–0.000 |

**River.** Both true-on records contain host and reporter clocks only. The runner lifetime upper bound comes from the canonical ledger. The process split is unidentifiable.

## Collection decision

The host comparison is resolved for this experiment. A corrected supplemental collection is needed to distinguish Mesh Jobs, application PostgreSQL and broker CPU. The local candidate exercises bounded roster changes and authenticates live PostgreSQL sessions at both ends of each sample. It uses a persistent diagnostic SQL client, authenticates its own backend by server-reported PID, preserves other unknown logins as unclassified, and joins the two session snapshots to same-UID pidfds, kernel start ticks and process clocks. Accepted samples use the original 25 ms cap. A mid-sample retirement drops that PID while preserving the other observed identities.

The ARM64 Linux fixture uses PostgreSQL 17.11, verified TLS and SCRAM. Its runner witnesses are owned fixture processes. The identity and lifecycle checks use real kernel and database identity, and the timed fixture reports its acceptance separately. A two-core run accepted all 120 frames and later runs recorded deadline rejections. The spare-core fixture also recorded a deadline rejection. Native adapter integration and matched workload overhead remain open. There is no supplemental controller request. Native-02 and the controller's held staging paths remain preserved. [Linux receipts](evidence/linux/README.md)

## Replay

Run from the checkout root.

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-ordinary-native-cpu-recovery
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
(cd "$packet/evidence/native-02" && shasum -a 256 -c ../../NATIVE-02-SHA256SUMS)
(cd "$packet/evidence/native-02" && shasum -a 256 -c ../../SOURCE-NATIVE-02-SHA256SUMS)
python3 -B "$packet/replay/check_recovery.py" -v
python3 -B "$packet/replay/recover.py" "$packet/evidence/native-02" \
  target/ordinary-native-cpu-recovery/verify-decisions.json
cmp "$packet/decisions.json" target/ordinary-native-cpu-recovery/verify-decisions.json
```

The Linux replay uses the checkout's local VZ boot helper, kernel, PostgreSQL image and Python overlay. Run it with a fresh output directory.

```sh
python3 -B "$packet/replay/local-linux.py" target/ordinary-native-cpu-recovery/linux-replay
```

The portable series keeps every original raw archive and replay input. Its hash manifest covers the intact original files and the copied release receipts. The original source directory remains in the checkout's target tree.
