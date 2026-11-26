# Ordinary FILE progress at the matched peer deadlines

At the deadline for 20% higher throughput than Oban, 20–32 of the 64 Mesh jobs still lacked even their first claim stamp in every historical FILE rotation. Crediting the entire 94.459 CPU-ms span-renderer component saving as an elapsed advance would still leave 9–24 jobs without a stamp. That makes late grant progress a material target even if every subsequent handler and settlement got faster.

For each measured rotation, hold FILE's observed durable enqueue fixed. Compute both the peer's submit and drain durations from the durable ledger, multiply each by 5/6, and take the earlier deadline on FILE's clock. Drain binds in all ten Oban and all ten River pairs. At equal throughput drain also binds in every pair. All 30 raw hashes match the released scores. This is the historical ordinary runner and original FILE broker at 4 MiB. The new cursor pilot has its own prospective controls.

| Storage | Block | FILE drain past Oban 1.20× deadline (ms) | Jobs unstamped | Jobs settled | Jobs still unstamped 94.459 ms later | FILE drain past River 1.20× deadline (ms) |
|---|---:|---:|---:|---:|---:|---:|
| DEFAULT | 1 | 229.843 | 25 | 32 | 16 | −88.770 |
| DEFAULT | 2 | 247.720 | 24 | 32 | 16 | −99.327 |
| DEFAULT | 3 | 249.122 | 25 | 32 | 16 | −44.103 |
| DEFAULT | 4 | 236.238 | 24 | 32 | 16 | −44.756 |
| DEFAULT | 5 | 215.609 | 21 | 40 | 9 | +26.634 |
| EXTERNAL | 1 | 279.555 | 25 | 32 | 16 | −20.232 |
| EXTERNAL | 2 | 244.160 | 28 | 32 | 16 | −126.762 |
| EXTERNAL | 3 | 208.675 | 25 | 32 | 13 | −123.466 |
| EXTERNAL | 4 | 439.119 | 32 | 24 | 24 | +171.091 |
| EXTERNAL | 5 | 212.278 | 20 | 40 | 12 | −61.949 |

The EXTERNAL fourth rotation also distinguishes the two peer targets. To beat River by 20%, Mesh needed 171.091 ms on drain. Six jobs on the measured FILE timeline still lacked a stamp 94.459 ms after that deadline. The corresponding DEFAULT miss was 26.634 ms, with three jobs still unstamped at the deadline. Each of the other eight River pairs already cleared both clocks.

The credit shifts the historical cutoff for inspection. It assumes the full serial CPU saving appears on the critical path before the affected work. Actual effects on admission order, overlapping work and CPU need fresh measurements. The stage counts compare a job's floored millisecond stamp, handler entry, two distinct durable writes, and settlement to the cutoff. A stamp whose rounding could change its classification is recorded separately. Owner counts identify the PIDs that eventually handled those jobs. The ledgers have no ordinary-cursor checked receipt timestamp per job.

[Offline reducer](ordinary_cursor_deadlines.py) · [Per-rotation stage and owner counts](ordinary-cursor-deadlines.json)
