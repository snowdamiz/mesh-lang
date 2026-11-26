# Original file-session timing stopped at a block boundary

The original TIMING grant released with exit 1 at 00:22:58 UTC.
Two blocks finished and exported 28 arms before the next assignment
check observed the hold. Each of the 12 treatment arms failed before
launch because its inventory referenced the parked adapter path. The
16 stock and peer arms returned 0. No matched block reconciled.

The [release](controller-release.json) and [raw summary](summary.json)
retain each arm's archive digest, exit status and failure signature.
The retry preflight replays the recovered proof and independently
checks the path failure for all three treatment lanes.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-public-large-file-session/resume/adapter-path-retry/retry.py check
```
