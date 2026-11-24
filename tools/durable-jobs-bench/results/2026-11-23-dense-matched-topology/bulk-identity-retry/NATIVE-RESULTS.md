# Dense bulk producer with the original two-runner consumer

Two opposite orders on the Linode used the same trusted V1 consumers, 64 total slots and two CPUs. In each arm the producer committed 1,024 jobs in one enqueue transaction. Each job completed two independent application writes and an individual terminal commit. Both arms used the same migration.

| Order | Producer | Enqueue, ms | Submit to settlement, jobs/s | Commit to settlement, jobs/s |
|---|---|---:|---:|---:|
| 01 | scalar | 517.53 | 554.03 | 769.50 |
| 01 | bulk | 177.99 | 678.45 | 769.15 |
| 02 | scalar | 511.54 | 549.03 | 756.51 |
| 02 | bulk | 171.19 | 658.27 | 739.66 |

The producer used eight set-based statements in place of 1,024 scalar calls. Bulk saved 340 ms to durable enqueue in each order. Submit-to-settlement throughput rose 22.5% and 19.9%. Post-commit throughput was slightly lower with bulk in each pair. This comparison isolates the producer on the original V1 runner; the public dense hot-pool comparison already uses a bulk producer.

The previous attempt stopped before submit on Linux denying access to each direct child’s executable link. This retry reuses the existing identity verifier and joins each live runner to its pinned launch file, argv, parent, stable start time, CPU mask, and ledger. Both attempts and all eight successful raw exports are retained. Run `python3 -B controller.py replay` from this directory to verify them.
