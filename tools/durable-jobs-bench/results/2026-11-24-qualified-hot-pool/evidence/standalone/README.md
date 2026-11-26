# Isolated Linux AMD64 compiler test for 1e70f18d

The standalone Linode grant released with exit 0 at 23:30:16 UTC on
2026-11-24. Cargo built the exact source on CPUs 0 and 1 and ran only
`the_performance_gate_measures_and_reports` with one test thread. The
test passed. Its assertions check the 200-iteration reporting path and
the result of an intentionally unattainable budget.

The raw archive contains the compiler ELF identity, the compiler hash
verified before and after the test, the hashed test and budget inputs,
and the native test log. See the [summary](summary.json) and
[controller release](controller-release.json).

From the repository root, replay the archived witness.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-qualified-hot-pool/controller.py standalone replay
```
