# Rejected SLOW PGLZ readiness pilot

The official pilot released exit 1 at 20:20:06Z. All 24 arms completed and both mirrored blocks failed control stability. Block 1 failed the warmup stock-to-lazy bridge on both official clocks. Both blocks failed the measured A/B spread and the measured bridge on both clocks. Confirmation and retry stayed closed. [Official release](../evidence/pilot/controller-release.json) · [Controller gate results](../evidence/pilot/replay.json)

The archive preserves all 145 pilot files, including the 24 raw archives and three release-pinned custody files. The archive manifest includes ten previously ignored logs. A credential-pattern scan of all 145 outer files and 892 native archive members found no matches. The prior proof logs restored in `2d527a199` are retained with their original bytes. [Pilot file manifest](archive.sha256)

## Where the lazy duplicate gaps accumulated

The table subtracts the faster lazy arm from the slower one within each block and phase. Each row telescopes to the submit-to-settlement gap. A negative interval means the slower arm spent less elapsed time in that interval. All figures are milliseconds.

| Block and phase | Slower lazy | Submit → enqueue | Enqueue → first write 64 | Write 64 → 448 | Write 448 → 512 | Write 512 → last effect | Last effect → terminal | Submit gap |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 warmup | A | +14.477 | +25.644 | −44.790 | +1.446 | −0.591 | +10.145 | 6.331 |
| 1 measured | A | −11.183 | −7.763 | +124.817 | −3.494 | −0.160 | +151.295 | 253.512 |
| 2 warmup | A | +52.614 | +12.827 | +110.738 | +2.888 | −3.443 | −171.066 | 4.558 |
| 2 measured | B | +21.478 | −47.503 | +101.402 | +29.846 | +0.043 | +82.801 | 188.067 |

The slower measured arm had already reached first-write rank 64 **18.946 ms earlier in block 1** and **26.025 ms earlier in block 2** on its own submit clock. Both fell behind before rank 448 and then incurred a longer terminal tail. Last first write to last effect differed by 0.160 ms and 0.043 ms in absolute terms. All four measured lazy arms had two distinct terminal XIDs after their last effect, with 3 versus 56 jobs in the final commit in block 1 and 29 versus 68 in block 2 for the slower and faster arms respectively. The warmup duplicate clocks were close in block 2 because the 110.738 ms middle difference and 171.066 ms opposing tail difference largely offset. [Per-arm landmarks and XIDs](landmarks.json)

## Workload and deployment clocks

The table shows all measured arms. Ranks and last effect are relative to submit. Drain starts at the durable enqueue commit. Launch starts at the rebuilt River first-runner launch, the retained River adapter dispatch including Go migration, or the first runner fork for Mesh and Oban. The archive also records an adapter-start clock for every arm, the host recording bounds, and the same landmarks for all 12 warmups.

| Block | Lane | Submit → terminal | Drain | First write 64 | First write 448 | First write 512 | Last effect | Terminal tail | Launch → terminal |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | Mesh V1 | 1749.904 | 1256.398 | 598.865 | 1495.892 | 1639.763 | 1741.247 | 8.657 | 4932.522 |
| 1 | Retained River | 1785.655 | 1296.474 | 801.805 | 1532.500 | 1639.910 | 1740.923 | 44.732 | 4924.007 |
| 1 | Lazy A | 2018.872 | 1547.093 | 775.270 | 1600.171 | 1707.272 | 1808.383 | 210.489 | 5069.873 |
| 1 | Warm River | 2028.014 | 1536.908 | 572.356 | 1577.164 | 1711.038 | 1812.600 | 215.414 | 5490.938 |
| 1 | Lazy B | 1765.360 | 1282.398 | 794.216 | 1494.300 | 1604.895 | 1706.166 | 59.194 | 4836.505 |
| 1 | Oban | 1839.761 | 1360.042 | 768.327 | 1608.481 | 1735.104 | 1838.636 | 1.125 | 6241.713 |
| 2 | Oban | 1902.627 | 1359.583 | 804.762 | 1661.478 | 1793.888 | 1898.783 | 3.844 | 6399.538 |
| 2 | Lazy B | 2005.682 | 1503.897 | 851.451 | 1642.603 | 1789.314 | 1890.617 | 115.065 | 5079.239 |
| 2 | Warm River | 1779.858 | 1292.782 | 566.089 | 1391.073 | 1520.248 | 1621.750 | 158.108 | 5221.427 |
| 2 | Lazy A | 1817.615 | 1337.308 | 877.476 | 1567.226 | 1684.091 | 1785.351 | 32.264 | 4888.729 |
| 2 | Retained River | 1838.173 | 1322.173 | 879.382 | 1588.609 | 1690.900 | 1792.105 | 46.068 | 4982.863 |
| 2 | Mesh V1 | 1774.076 | 1272.892 | 601.214 | 1527.251 | 1668.828 | 1769.956 | 4.120 | 4960.663 |

Warm River advanced the 64th first write from enqueue commit by 226.123 ms and 294.405 ms against the measured lazy midpoint. It finished 135.898 ms later from submit in block 1 and 131.791 ms earlier in block 2. From the common rebuilt first-runner launch, it finished **537.749 ms later in block 1 and 237.443 ms later in block 2**. Its pre-submit interval was 401.851 ms and 369.234 ms longer, mostly before the ready observation. These are descriptions of rejected pairs. [All workload and startup clocks](landmarks.json)

## Bounds of the host evidence

Each submit-to-terminal window had 9–11 bracketing samples, including both endpoints. The maximum adjacent gap was 250.122 ms. The outer brackets included 46.416–420.285 ms beyond the workload window. The lower and upper figures below use cumulative deltas between sample labels fully inside the interval and between its outer bracketing labels. CPU I/O wait is summed across CPUs 0 and 1 in 10 ms ticks. Disk busy is for the whole recorded device. The worker reads CPU stats before stamping the sample and disk stats afterward. Individual read times and sub-sample attribution are unavailable. [Host sampler](../../../worker.py) · [Per-window bounds](landmarks.json)

| Measured lazy arm | Submit → terminal I/O wait, CPU-ms | Submit → terminal disk busy, ms | Write 64 → 448 I/O wait, CPU-ms | Last effect → terminal fully contained sample intervals |
| --- | ---: | ---: | ---: | ---: |
| 1 A, slower | 30–30 | 112–112 | 10–20 | 0 |
| 1 B, faster | 30–50 | 119–129 | 10–20 | 0 |
| 2 A, faster | 60–60 | 148–151 | 20–50 | 0 |
| 2 B, slower | 180–200 | 358–377 | 70–170 | 0 |

The block 2 slow duplicate coincided with higher sampled I/O-wait and device busy totals across its workload. Block 1 has no matching slow-arm I/O-wait pattern. No measured lazy tail has a complete interval between two interior host samples. Recorded steal deltas were zero in all arms. The snapshots do not identify a process, SQL operation, or cause for the sample differences. Mesh's four submit times stayed between 1749.904 and 1801.938 ms even though River lanes appeared in both faster and slower ranges. The observed modes alone do not establish a host-wide cause. [Per-arm host and workload evidence](landmarks.json)

## Archive arithmetic

```sh
d=tools/durable-jobs-bench/results/2026-11-25-slow-river-readiness-timing/pilot-analysis
python3 -B "$d/reconcile.py" | cmp - "$d/landmarks.json"
```

The reader verifies the official release, all raw and custody hashes, per-arm request identity, and the rejected order and gates. It recomputes all 24 ledgers' durable stages and launch clocks, and checks them against the recorded scores. It reads archived files only.
