# Matched SLOW PGLZ readiness timing

This study compares genuine slot-ready River with lazy River and the frozen Mesh V1 and Oban peers. It uses the seven-arm corrected replay as a prerequisite. The original archived qualification includes the expected pre-submit rejection at 66 clients. The timing study uses six positive lanes and excludes the shortage arm.

Every arm provisions its own fresh PG17.11 cluster on the same host boot, kernel, disk and CPUs 0 and 1. It runs 512 sealed 16 KiB jobs with 100 ms holds, two independent application commits per job, and two 32-slot runner processes under cap 71. Storage remains PGLZ, Mesh remains unfused V1, and Oban uses its ordinary matched image. The retained River bridge uses the original lazy ELF. Lazy A, warm River and lazy B share the qualified `b591ed46…` ELF. The only timing-stage source changes are admitting warmup and measured phases and removing the shortage mode from the shell wrapper. The controller checks the same binary and each lane's effective inventory before dispatch.

The pilot uses two exactly mirrored rotations. Four confirmation rotations form two more mirrored pairs. The six measured rotations put each lane in every position once and balance immediate neighbours. Each rotation repeats its order for fresh unscored warmup and fresh measured arms. Each warm arm pays the full fill and readiness cost in its own deployment, including the common idle interval. Rebuilt lazy reports that same launch boundary. Report producer connection setup, runner launch to readiness and settlement, adapter start through last settlement, and the full recorder envelope. The official clocks start at submit and durable enqueue commit. Preserve durable first-write ranks 64, 448 and 512, the last effect and the terminal tail.

Every warmup and measured arm must reconcile its durable ledger, two application XIDs per job, real readiness, physical cap, PostgreSQL pins and CPU/host evidence. Warm River needs 67 physical clients through submit and stable detached listeners. Lazy needs seven at submit. The noise gates are at most 1% CPU steal and 5% I/O wait, valid disk counters and no overlapping builds. On each official clock, the lazy A/B spread must be no greater than the larger of 75 ms and 5% of their midpoint. The retained River bridge must differ from each rebuilt lazy arm by no more than the larger of 100 ms and 7.5% of retained elapsed. Both pilot blocks must pass these gates and replay from the canonical release before confirmation. Advancement depends on control quality.

Report the lazy midpoint minus warm River, warm River minus Mesh, and Oban minus Mesh as paired elapsed differences. An official-clock directional claim requires all six differences to agree in sign and individually clear both 5% of the relevant peer elapsed and the largest absolute bridge or lazy duplicate difference in that block. Apply the same guard to the adapter-start clock and report the rebuilt launch-only differences separately. Also report whether each matched block clears the separate 20% throughput cutoff, `peer elapsed > 1.2 × Mesh elapsed`. Diagnostic proof latencies stay out of the timing estimates.

## Sole controller commands

Use the canonical checkout and the ordinary controller queue. This packet is source-only. The timing driver cannot enroll itself.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-slow-river-readiness-timing
python3 -B "$d/controller.py" check
python3 -B "$d/controller.py" pilot
python3 -B "$d/controller.py" replay-pilot
python3 -B "$d/controller.py" confirm
python3 -B "$d/controller.py" replay-confirm
```

The 24-arm pilot stages its fresh timing code directory under the existing native locks. The 48-arm confirmation audits the same retained timing files and candidate before dispatch. Engine and PostgreSQL file pins are checked again before each phase and by each worker. Both releases bind their exact request, the complete `raw_sha256.json`, and a `retained_custody_sha256` map for `public-code-access.log`, `retained-native-sha256.log` and `retained-inputs.json`. Each phase has its own local evidence and remote worker paths. A failed or rejected phase retains its evidence and keeps confirmation gated.
