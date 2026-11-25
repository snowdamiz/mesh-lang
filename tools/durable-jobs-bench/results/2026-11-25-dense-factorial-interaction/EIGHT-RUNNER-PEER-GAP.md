# River gains most after the first 64 dense effects

In both matched 8×8 rotations, public grouped hot Mesh reached its 64th durable effect sooner than River after enqueue commit. River then completed the remaining 960 effects **468 ms and 417 ms faster**. River spent longer on its final settlement tail. Its resulting submit leads were 225 ms and 222 ms. Mesh finished ahead of Oban by 82 ms and 208 ms. The reverse block's duplicate-control spreads were 272 ms from submit and 268 ms from commit, leaving both peer comparisons inconclusive under the sealed rule. Apalis cleared its lead in both blocks.

| Elapsed phase, Mesh minus River (ms) | Forward | Reverse |
|---|---:|---:|
| First submit to durable enqueue commit | +85 | +98 |
| Enqueue commit through 64th effect | −181 | −95 |
| 65th through 768th effects, starting at effect 64 | +285 | +274 |
| Remaining 256 effects, starting at effect 768 | +184 | +143 |
| Last effect through final settlement | −146 | −197 |

With each observed drain held fixed, an instantaneous Mesh producer would still leave Mesh 46 and 41 ms behind River from submit. Reaching 1.20× River's submit throughput under that favorable assumption would require another 258 and 241 ms off drain. Those figures price the target. An actual producer change can alter the drain schedule.

The first complete rotation comes from the preserved exit-1 release. The fresh reverse rotation comes from the successful continuation. Each arm completed 1,024 jobs and two distinct application commits per job. Run `python3 -B eight_runner_peer_gap.py | cmp - eight-runner-peer-gap.json` in this directory to check the eight measured comparison archives in owner commit `893710708` and reproduce both clocks. [Unrounded phase durations and hashes](eight-runner-peer-gap.json) · [Runner availability in the pilots](RUNNER-WORK.md)
