# Released matched dense 2×32 peer blocks

The sole controller released five fresh matched Linode blocks with exit 0
at 03:58:03 UTC. Each block ran the released grouped-refill Mesh ELF in
fixed8ms mode with the exact logical-key grant alongside River, Oban and
Apalis at two runners of 32 slots. All used CPUs 0 and 1, 1,024 jobs,
two independently committed application writes per job and one durable
enqueue. All 40 peer warmup and measured arms reconciled. The final replay
also includes the 22 native factorial archives. [Peer release](evidence/native-retry/peers-release.json) ·
[Full replay](evidence/native-retry/final-replay.json) ·
[Evidence hashes](evidence/native-retry/SHA256SUMS)

Measured medians are over five blocks. Submit starts at real enqueue
submission. Drain and last effect start at enqueue commit. CPU figures
are CPU-ms. PostgreSQL process CPU was bracketed on Mesh throughout the
factorial and peer series. The stock peer inventories provide runner and
host CPU for matched comparisons.

| Engine | Submit ms | Drain ms | Last effect ms | Runner CPU | Host CPU | Terminal XIDs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Mesh composition | 1,003.271 | 841.442 | 835.089 | 519.712 | 1,880 | 161 |
| River | 1,021.191 | 945.403 | 744.960 | 448.411 | 1,680 | 12 |
| Oban | 1,321.836 | 1,184.883 | 1,183.194 | 3,133.711 | 2,460 | 1,024 |
| Apalis | 739.792 | 701.974 | 698.417 | 354.542 | 1,450 | 56 |

Paired deltas below are the median of within-block differences.

- Mesh beat River by 21.194 ms on submit in four of five blocks and by
  101.467 ms on drain in all five. River reached the last durable effect
  77.230 ms earlier and used 220 fewer host CPU-ms at the paired medians.
- Mesh beat Oban by 327.241 ms on submit and 346.574 ms on drain,
  winning all five blocks on each clock.
- Apalis beat Mesh by 266.027 ms on submit and 131.626 ms on drain,
  winning all five blocks on each clock.

The pilot and peer decisions are scoped to this build-free composition.
The prior grouped-refill and logical-key peer decisions remain closed.
[Pilot results](PILOT-RESULTS.md)
