# Historical strict LARGE comparison

The separate strict `955b356b` comparison released exit 0 at 02:21:42 UTC.
Five rotated groups reconciled all 40 warmup and measured arms. Each arm
used 64 jobs with 512 KiB payloads and two pinned CPUs. The measured
lanes used the exact candidate, historical stock Mesh, Oban, and River.
[SETUP and artifact binding](../2026-11-23-large-v3-setup-955b356b) ·
[TIMING release](controller-release.json) · [Raw replay](summary.json)

Median submit-to-settlement p99 was 2,665.197 ms for the candidate
and 2,645.968 ms for stock Mesh. Median first submit to commit was
1,887.029 ms for the candidate and 1,804.358 ms for stock Mesh.

```bash
python3 -B tools/durable-jobs-bench/interleave.py summarize 2026-11-23-large-v3-timing-955b356b
```
