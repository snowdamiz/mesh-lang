# Build-free handler-pool timing retry

The original timing warmup started both pinned runners and reached
readiness, then Linux rejected the unprivileged `/proc/PID/exe` check.
The [first release and raw arm](../evidence/timing) remain archived.

The retry uses the existing direct-child identity verifier. It binds the
pre-launch and ready executable digest and file stamp, direct parent, live
argv, stable process start tick, and CPU affinity. When Linux permits the
exe symlink read, it must match the exact path. A denied read is explicit in
the receipt. Replay checks either method and runs the original workload
scorer. All four qualified candidate ELFs, peers, workloads, rotations,
and timing windows use their original pins. No native build is required.

```bash
python3 -B controller.py check
python3 -B controller.py replay
```
