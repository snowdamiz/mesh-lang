# Combined scheduler and exact Jobs layout

The GNU AArch64 Jobs ELF `f14316c56d193cedd74b9d1b2be1940eff2d4844cd5532750dcae773885c17f4` has mixed slow64 results. Both of its control series passed the frozen aggregate in the extra-cut experiment. The preselected primary in the subsequent five-pair replication missed throughput and pickup p99. These measurements apply to this isolated ELF and workload. Ordinary packages use the standard linker path.

## Slow64 results

Each arm ran 512 warmup jobs and 512 measured jobs against a fresh PostgreSQL 17.11 database with durable commits and TLS. The measured workload used 64 slots, 100 ms holds, and a 68-connection cap. Oban Basic 2.24.2 used the stock `8526a5bd` fixtures, file-only resolution, Erlang 29.1.1, Elixir 1.20.4, and `+S 2:2 +SDcpu 1:1 +SDio 1 +A 2`. Measured Mesh arms ran trace-off with the worker-only THP flag. The shared 15-core Apple Silicon host ran two-vCPU GNU Linux ARM64 guests with 2 GiB RAM, 4 KiB pages, and a 64 KiB fault window. Every timed arm recorded 512 ordered, distinct-XID write pairs and a clean runner drain. The receipt keeps each arm's load, original score, and shutdown peaks.

| Series and arm | Median jobs/s | Median pickup p99 (ms) | Measured RSS (MiB) | Through shutdown (MiB) | Aggregate misses in both views |
| --- | ---: | ---: | ---: | ---: | --- |
| Earlier pilots, `f14316c5` candidate | 493.83 | 919.54 | 218.22 | 218.22 | Peak RSS |
| Earlier pilots, Oban | 363.32 | 1,295.79 | 142.98 | 143.05 | |
| Extra-cut, `f14316c5` control | 324.84 | 1,424.28 | 211.92 | 211.92 | None |
| Extra-cut, `f14316c5` repeat | 372.12 | 1,259.28 | 217.30 | 217.30 | None |
| Extra-cut, `882826a9` primary | 295.30 | 1,623.95 | 195.24 | 195.24 | Throughput |
| Extra-cut, Oban | 311.18 | 1,523.43 | 145.71 | 146.91 | |
| Replication, `f14316c5` primary | 231.80 | 2,076.77 | 214.33 | 214.33 | Throughput and pickup p99 |
| Replication, `f14316c5` repeat | 530.68 | 842.84 | 218.26 | 218.26 | Peak RSS |
| Replication, Oban | 359.42 | 1,318.54 | 144.30 | 144.30 | |

Both earlier `f14316c5` pilots reached 64 committed overlaps. They used 217.59375 and 218.84375 MiB against shutdown-inclusive caps of 216.00586 and 213.140625 MiB. The misses were 1.59 and 5.70 MiB. Both extra-cut control series reached 64 committed overlaps in all five groups. The extra-cut primary reached 57 in the last boot. Its throughput miss and overlap loss both count.

The replication froze the primary, byte-identical repeat at the same guest path, Oban arm, and all five orders before boot one. The primary and Oban reached 64 overlaps every time. The repeat reached 53 in pair two. All groups count in the original and shutdown-inclusive scores. The supplemental full-overlap check also failed. Host one-minute load ranged from 21.09 to 95.82 in the replication, with substantial within-boot variation. The matched duplicate rates span both directions. These trials leave the end-to-end throughput difference unresolved and support no slow64 performance qualification for `f14316c5`. [Frozen order](replication-freeze.json) · [Per-arm metrics and score digests](slow64-receipts.json)

From the checkout root, use Python 3.11 or later to recalculate the frozen scores and independently sweep the half-open committed hold intervals. The CSV archive contains relative integer microseconds from both effect commits for every measured job. The replay verifies all 43 measured arms and both score views against the original reduced receipts.

```sh
python3 -B proof/jobs-parity/linux-integrated-uncut/replay-scores.py
```

The retained host driver source is [replication.py](replication.py). The freeze records its original location under `target/integrated-uncut-delivery` and pins the archived guest inputs it uses. Run the checkout score replay to verify the evidence without those inputs.

## Scheduler behavior

The patch starts scanning immediately, publishes idle, and scans again before parking. Shared normal and high-priority producers queue before checking the idle flag. Grouped producers queue before unconditionally signaling the owner. Work published before the idle flag is visible to the second scan. Work published after the idle flag receives an unpark permit, including when the permit arrives before the worker enters `park_timeout`. The worker installs its thread handle before advertising idle. The shutdown store follows the same wake protocol.

The 16 ms timeout applies before the first dispatch when one worker is configured, there are zero managed actors, movable steps are disabled, and shutdown is clear. The first dispatch switches the worker back to the ordinary spin budget and 1.1 ms polling. The existing elastic bounds, retired worker polling, movable-step rescue, and suspended actor sweeps use their existing timeouts. A snapshot includes elapsed time in an extended park before it finishes. The autoscaler still reads runnable actor counts.

In the earlier ordinary one-worker Linux screen, the same scheduler patch saved a median 567 µs of process CPU during a 100 ms idle interval, against 89 µs of duplicate-control spread. It added about 13 µs to first-actor dispatch and 17 µs of process CPU to immediate exit. The tradeoff was measured on the earlier standalone fixture. The persistent two-worker workload showed no consistent improvement beyond duplicate-control spread.

The exact two-file patch is local commit `f8f8ed21cef0dc59fc7683bc873bac896e57069a`. The 19 local source gates passed on macOS ARM64. They include formatting, workspace build, core and compiled tests, fuzz compilation, examples, policy laws, pinned Z3, and the live PostgreSQL 17.11, 18.7, and 19.0 compatibility and effect-fence gates. A fresh GNU Linux boot ran 15 compiled actor, timer, GC, and clean exit checks against the rebuilt runtime with fixed 1/1, fixed 2/2, and elastic 1/2 worker bounds. [Source verification](source-verification.json)

## Relink the frozen executable

Use the ARM64 macOS Zig 0.15.2 driver and pinned GNU AArch64 inputs from the manifest. All paths in the manifest are relative to the retained input root. You can move that root or override the runtime and Zig paths. The recipe verifies the input hashes and runs LLD in the fresh output directory with literal input basenames. It emits `mesh-stock` with the original unarranged linker settings. With `--layout`, it also emits `mesh` after verifying the original map, the generated order and the placed order. The profile selects 1,594 read-only contributions before `.rodata` and 2,255 text contributions after `.text`. Each entry identifies a single object or archive member and section. The order is specific to this fixture and archive. The generic observed-section tool is a separate layout path.

```sh
proof=proof/jobs-parity/linux-integrated-uncut
lane=target/integrated-uncut-delivery
python3 -B "$proof/relink.py" "$lane/relink-stock"
python3 -B "$proof/relink.py" "$lane/relink-profiled" --layout
```

A fresh locked release build of `mesh-rt` from the combined source produced archive SHA-256 `fcb9f02c93e1ae519f9471212aa4b1c938db244908f6e976db9eb240c6865a82`. The build uses Rust 1.99.0, the original Cargo registry source path, and fresh Zig caches. The archived build embeds source paths in some dependencies, so the exact archive needs the original registry path. The relink can use the pinned archive from any input root. The fresh archive and retained archive independently produced identical unarranged and profiled ELFs, order scripts, and placed-order receipts. [Relink result](relink-result.json)

The manifest pins the frozen compiled application object, native constructor archive, runtime, driver, CRT, and link-time DSOs. The Mesh fixture sources are in [fixture](fixture). The native source is [worker_identity.c](../../../tests/e2e/durable_jobs/.parity/worker_identity.c). The archived compiler embeds input paths in the object. Use the pinned object for exact bytes. [Source pins](source-pins.json) · [Link-input manifest](link-inputs.json) · [Ordered indices](section-order.json)

Run the package fault proof in the pinned GNU Linux guest. The runner and its fresh workers use the same verified `f14316c5` executable. The lost-acknowledgment proof passes TLS end to end. An isolated loopback relay withholds the attested application session's replies while a separate verified TLS connection observes the commit.

```sh
python3 -B "$proof/run-package.py" "$lane/relink-profiled/mesh" "$lane/package-proof"
```

The traced hold boot executed the real worker constructor, then matched 64 fresh worker execs, zero-status exits, and exact consuming waits to the durable ledger. The compiler fixture recorded two distinct XIDs per job, 128 per-call resets, and eight startup resets. The direct constructor probe recorded one marker and exit 174 without a protected effect channel. The wrong settlement fence was rejected. [Trace receipt](traced-lifecycle.json)

The fresh fault boot proved recovery on the relinked ELF. Stop drained the held handler with both commits. The deadline left the first commit durable and discarded the attempt. Lease renewal advanced the existing claim, then owner loss led to a fresh attempt and rejection of the stale fence. The lost response left the first commit in place even though the handler returned complete. The runner retried and the replacement preserved that XID. The TLS listener recorded one notification scan and zero fallback scans. [Fault and LISTEN receipt](package-result.json)
