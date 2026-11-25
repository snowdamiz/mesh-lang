# Released native handler application-pool comparison

Both Linode timing windows released with exit 0 on November 25.
Two offline reductions agreed on all six dense and six short blocks,
with 108 warmup and measured arm exports. The four linked fixture
ELFs and pool-scope witness were qualified by the
[earlier SETUP](../../evidence/setup/app-pool-39e79f4abb76).
All arms verified the helper reads and owned-child identity.

Against the faster service duplicate, the direct handoff saved a
median of 44.475 ms on dense drain (range 34.002–70.396 ms)
and 30.450 ms on full submit-to-settlement (range −93.200 to
84.365 ms). Three of six dense blocks beat the duplicate spreads on
all required clocks, below the required five, and the median drain
fell below the 62.4 ms floor. The short profile cleared the clock
spreads in one of six blocks. The adoption gate stayed closed.

See [groups 1–2](controller-release-groups-1-2.json),
[groups 3–6](controller-release-groups-3-6.json), and
[full replay](summary.json).

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-trusted-handler-app-pool/timing-clean-identity-retry/controller.py replay
```
