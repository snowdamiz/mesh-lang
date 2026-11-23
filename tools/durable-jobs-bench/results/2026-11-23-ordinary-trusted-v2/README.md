# Ordinary trusted V2 short replay

This packet archives the frozen November 23, 2026 experiment and its
native qualification. Both ordinary lanes used the same AMD64 runner ELF
`cbd23c92eba11f9237c11ae6a5fc60128e1852956f4be21913f8e7f716b25a5f`.
The application still commits two independent writes per job. The success
cohort lane uses set-based admission and can group up to eight successful
terminal rows in one transaction inside the trusted shared process contract.
The protected Mesh control still runs fresh workers with fenced effects and
one terminal transaction per job. [Exact build and peer pins](evidence/ordinary-trusted-v2-short/series.json)

In the first matched two-CPU block, 128 jobs per arm ran on PostgreSQL
17.11 with verified TLS. The success cohort reached 527.39 jobs/s from
first submit and 728.70 from first commit. The same-image ordinary stock
lane reached 436.72 and 569.21. The submit rates for the fresh peer controls
were 199.68 for protected Mesh, 298.04 for Oban, 715.12 for River, and
944.09 for Apalis. The packet includes the measured arms and warmups for
this block. [First block and its raw hashes](evidence/ordinary-trusted-v2-short/summary-partial.json)

Run the driver from the checkout root with `PYTHONPATH` pointing at the
benchmark suite. Its summary selects the same retained block offline and
reports the four missing rotations with exit status 1.

```sh
suite=tools/durable-jobs-bench
archive="$suite/results/2026-11-23-ordinary-trusted-v2"
PYTHONPATH="$suite" python3 -B "$archive/replay/interleave.py" summarize ordinary-trusted-v2-short
(cd "$archive" && shasum -a 256 -c SHA256SUMS)
```
