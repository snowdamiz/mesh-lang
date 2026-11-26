# Dense submit and drain budgets at two Mesh runners

Each rotation ran the frozen 1,024-job workload on two CPUs. Mesh used two 32-slot runners. Each peer used eight 8-slot runners. The calculations use the archived, reconciled clocks.

The full gap is Mesh submit-to-settlement time less the peer’s. The final column gives the minimum drain saving still needed if Mesh could enqueue instantaneously and preserve its measured drain. All values are milliseconds.

| Rotation | Peer | Mesh enqueue | Full gap | Drain saving needed with free enqueue |
|---|---|---:|---:|---:|
| 01 | Go River | 515.7 | +641.2 | 125.5 |
| 01 | Elixir Oban | 515.7 | +341.2 | 0.0 |
| 01 | Rust Apalis | 515.7 | +1028.6 | 512.9 |
| 02 | Go River | 517.4 | +635.1 | 117.7 |
| 02 | Elixir Oban | 517.4 | +272.2 | 0.0 |
| 02 | Rust Apalis | 517.4 | +962.9 | 445.5 |
| 03 | Go River | 536.8 | +653.3 | 116.6 |
| 03 | Elixir Oban | 536.8 | +127.0 | 0.0 |
| 03 | Rust Apalis | 536.8 | +921.7 | 384.9 |
| 04 | Go River | 510.0 | +703.0 | 193.1 |
| 04 | Elixir Oban | 510.0 | +255.6 | 0.0 |
| 04 | Rust Apalis | 510.0 | +939.8 | 429.8 |
| 05 | Go River | 505.7 | +602.8 | 97.2 |
| 05 | Elixir Oban | 505.7 | +205.4 | 0.0 |
| 05 | Rust Apalis | 505.7 | +959.2 | 453.5 |

| Peer | Rotations requiring faster drain | Median minimum saving |
|---|---:|---:|
| Go River | 5/5 | 117.7 ms |
| Elixir Oban | 0/5 | 0.0 ms |
| Rust Apalis | 5/5 | 445.5 ms |

A positive residual requires a consumer-side saving even under the free-enqueue assumption. A measured change to the enqueue path can also change consumer behavior. [Original crossover, controls and replay](../2026-11-23-dense-matched-topology/README.md)
