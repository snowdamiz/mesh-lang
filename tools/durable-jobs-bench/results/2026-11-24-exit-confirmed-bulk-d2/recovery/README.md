# Build-free recovery for c7c50e726ca0

The first SETUP linked candidate ELF `972514efac68` and then failed at `faults.py:117`. The nested source extraction overwrote `replay/wire.py`, `replay/split_arm.py`, and `replay/combined-run` after the approved versions were staged. The old Relay has no `requests` field. The original capture and both retained binaries remain available to the controller. [Original attempt](../evidence/exit-bulk-d2-c7c50e726ca0/)

The corrected transfer extracts the nested archive under `capsule/` and verifies all seven executable replay entries after extraction. The local Linux guest imported the staged module under UID and GID 65534, with zero capabilities, CPUs 0 and 1, and no bytecode cache. The old package reproduced the recorded exception. The repaired package recorded the probe and INSERT and dropped the committed INSERT receipt. Replacing the repaired module with the old bytes failed the digest audit. [Linux receipt](linux-regression.json) · [Serial transcript](linux-regression.serial)

## Replay the package check

From the checkout root, run the following commands.

```sh
r=tools/durable-jobs-bench/results/2026-11-24-exit-confirmed-bulk-d2/recovery
(cd "$r" && shasum -a 256 -c SHA256SUMS)
python3 -B "$r/recover.py" check
```

The offline check replays the archived Linux r2 fault and teardown proofs and the exact-source local D2 fault and teardown witnesses. The D2 callback witness is from macOS. A fresh native callback witness would require a separately linked fixture. The recovery uses the two existing Linux runners for the fresh PostgreSQL faults and short correctness smokes.

## Dispatch with the sole controller

Give `2026-11-24-exit-confirmed-bulk-d2-recovery` a SETUP grant. The recipe rechecks the archived candidate and baseline ELF and manifest digests, the retained broker, all 60 host-side pins, the boot, kernel, mount, clock ticks, PostgreSQL 17.11, and CPUs 0 and 1. It reruns the staged relay check, the four D2 faults, the three writer-off control faults, and the two writer-off matched short smokes. The smoke reducer requires the original 1 runner, 16 slots, 8 application connections, and connection cap of 20, along with the original bulk, application XID, queue, and process identity checks.

```sh
python3 -B "$r/recover.py" setup
python3 -B "$r/recover.py" verify-setup
```

After the successful SETUP release, grant TIMING under the original `2026-11-24-exit-confirmed-bulk-d2` ID. Use the original six lanes, five orders, result directory, and scorer. Run the last three groups after the first two reconcile.

```sh
python3 -B "$r/recover.py" timing 1 2
python3 -B "$r/recover.py" timing 3 4 5
python3 -B "$r/recover.py" summarize
```

[Controller packet](controller-packet.json)
