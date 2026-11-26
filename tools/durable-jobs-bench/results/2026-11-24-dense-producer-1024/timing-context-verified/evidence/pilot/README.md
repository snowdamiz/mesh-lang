# Verified producer-context pilot

Both native broker preflights entered and exited the retained broker
as UID and GID 65534 over verified TLS. Both recorded zero submissions.
The workload advanced to `capture_runners`, where `pinned_file`
rejected the noncanonical `packet-timing-context-verified/../broker/mesh`
path. The controller released exit 1 at 02:31:16 UTC.

All 12 attempted arms exported raw failure archives. Replay selected
zero groups and left the peer gate closed. The original SETUP and both
earlier failed pilots have separate evidence.
[Release](controller-release.json) · [Replay](replay.json) · [Native preflight](entry-preflight)

```bash
d=tools/durable-jobs-bench/results/2026-11-24-dense-producer-1024/timing-context-verified
python3 -B "$d/context_retry.py" replay pilot
```
