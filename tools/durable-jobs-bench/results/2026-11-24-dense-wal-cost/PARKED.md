# Parked for the next session

The controller accepted the sealed 16-arm intake behind the existing bookings. The matrix entered HOLD at 05:51:30 UTC on November 24 after releasing the public large EXTERNAL run. The WAL intake has no TIMING grant, dispatch, or native exports. Its existing reservation remains queued.

The sealed [controller packet](controller-packet.json) is byte-identical to `target/linode-jobs-stepchange/requests/2026-11-24-dense-wal-cost.json` with SHA-256 `7de1c8c9709b7912e96b4d3a91d4b5efb6e9c05f3b18b2e05b2ca95841240db3`. The proof and archived pricing are in [the diagnostic report](README.md) and [the archived device replay](ARCHIVE-REPLAY.md).

When the user resumes dispatch, the sole controller can use these commands from the checkout root. The timing entry requires the original packet and an active named grant. The replay expects the raw exports in this directory's `evidence/native/`.

```sh
p=tools/durable-jobs-bench/results/2026-11-24-dense-wal-cost
python3 -B "$p/controller.py" check
python3 -B "$p/controller.py" timing
python3 -B "$p/controller.py" replay
```

Archive the controller release, each raw attempt, any failures, and the replay result in Git. Keep the whole-workload PostgreSQL scope and the in-drain whole-device scope separate. The causal treatment remains gated on the native result and the booked concurrency experiments.
