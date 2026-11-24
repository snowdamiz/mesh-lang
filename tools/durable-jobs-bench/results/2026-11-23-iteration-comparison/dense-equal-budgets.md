# Dense submit and drain budgets at two Mesh runners

Each rotation ran the frozen 1,024-job workload on two CPUs. Mesh used two 32-slot runners. Each peer used two 32-slot runners. The calculations use the archived, reconciled clocks.

The full gap is Mesh submit-to-settlement time less the peer’s. The final column gives the minimum drain saving still needed if Mesh could enqueue instantaneously and preserve its measured drain. All values are milliseconds.

| Rotation | Peer | Mesh enqueue | Full gap | Drain saving needed with free enqueue |
|---|---|---:|---:|---:|
| 01 | Rust Apalis | 542.5 | +1120.5 | 578.0 |
| 01 | Elixir Oban | 542.5 | +565.8 | 23.2 |
| 01 | Go River | 542.5 | +854.9 | 312.3 |
| 02 | Rust Apalis | 527.9 | +1121.4 | 593.5 |
| 02 | Elixir Oban | 527.9 | +532.5 | 4.5 |
| 02 | Go River | 527.9 | +961.0 | 433.1 |
| 03 | Rust Apalis | 510.9 | +1041.9 | 531.0 |
| 03 | Elixir Oban | 510.9 | +422.2 | 0.0 |
| 03 | Go River | 510.9 | +805.6 | 294.7 |
| 04 | Rust Apalis | 502.9 | +994.8 | 491.9 |
| 04 | Elixir Oban | 502.9 | +543.3 | 40.3 |
| 04 | Go River | 502.9 | +806.3 | 303.4 |
| 05 | Rust Apalis | 545.0 | +1180.2 | 635.3 |
| 05 | Elixir Oban | 545.0 | +617.4 | 72.4 |
| 05 | Go River | 545.0 | +901.5 | 356.5 |

| Peer | Rotations requiring faster drain | Median minimum saving |
|---|---:|---:|
| Rust Apalis | 5/5 | 578.0 ms |
| Elixir Oban | 4/5 | 23.2 ms |
| Go River | 5/5 | 312.3 ms |

A positive residual requires a consumer-side saving even under the free-enqueue assumption. A measured change to the enqueue path can also change consumer behavior. [Original crossover, controls and replay](../2026-11-23-dense-matched-topology/README.md)
