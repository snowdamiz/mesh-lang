# Dense fixed8 River readiness sensitivity

The fenced Linode SETUP, two-block pilot and four-block continuation all released with exit 0. The final release was at 04:22:36 UTC on November 25, 2026. The sealed replay reconciled 52 raw archives covering four diagnostic arms and six measured blocks with their warmups.

The warmed River arms proved two 33-session runner cohorts and one observer at readiness and at submit. Sampled peaks ranged from 67 to 69 under the cap of 96. In the six measured blocks, same-image warming saved a paired median 241.358 ms on submit through settlement and 243.539 ms from first commit through settlement. The warmed River lane finished ahead of the fixed8 plus d388 Mesh lane in all six blocks, by paired medians of 272.638 ms on submit and 158.896 ms on drain. These clocks start with submission or the first commit. Connection setup and runner readiness are recorded separately in the replay.

Run `python3 -B tools/durable-jobs-bench/results/2026-11-25-dense-fixed8-river-readiness/controller.py replay` from the checkout root. The final replay is `replay-rest.json`. The three fenced releases are under `controller`. The earlier stock River peer comparison retains its own release and evidence.
