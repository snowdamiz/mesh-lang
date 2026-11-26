# What the FILE producer could cover on dense input

The five public dense 8×8 blocks had 1 MiB of application payload per submission. Closing River’s enqueue lead would require saving 99, 101, 134, 161 and 132 ms, respectively. A FILE producer removes the wrapper’s inline JSON conversion and pipe transfer. The released FILE comparisons involved 8 MiB or 32 MiB per submission and had fewer job records. [Dense clocks](../2026-11-25-public-dense-late-capacity/peer-comparison/RESULTS.md) · [Transport implementation](../2026-11-24-public-large-file-session/adapter.py)

| Released comparison | Jobs | Total payload | Paired saving to durable enqueue | Saving per payload MiB |
|---|---:|---:|---:|---:|
| LARGE FILE vs faster same-image inline duplicate | 64 | 32 MiB | 188–354 ms, median 290 | 5.9–11.1 ms |
| SLOW FILE vs faster 1×64 inline duplicate | 512 | 8 MiB | 8.6 and 24.2 ms | 1.1 and 3.0 ms |
| SLOW FILE vs single 2×32 inline arm | 512 | 8 MiB | 69.6 and 95.4 ms | 8.7 and 11.9 ms |
| DENSE Mesh inline deficit to River | 1,024 | 1 MiB | 99–161 ms, median 132 | 99–161 ms |

Transferring the largest observed per-byte saving to DENSE gives about 12 ms, or 9% of its median enqueue deficit. That calculation isolates a possible byte-proportional contribution. It provides no bound on the cost per job of encoding or decoding 1,024 records. The two SLOW shapes themselves gave very different savings on identical payloads. The evidence supports measuring the eight database calls before allocating a dense timing slot to FILE alone.

The replay reads 35 measured Linode captures across the three series, checks each raw digest against its scorer and verifies the successful releases. It regenerates all payloads from the frozen seed and matches each digest by job key. For every arm, it reconciles the one transaction’s returned ID map and both durable application XIDs per job. Byte counts cover the application payload fields only. The gate decisions in the original series continue to govern promotion.

Run from this directory to reproduce the unrounded timings.

```sh
python3 -B file_transport_price.py | cmp - file-transport-price.json
```
