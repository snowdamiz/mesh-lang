# Trusted Jobs product transfer at db5937e7

The exact product revision `db5937e7e8988f935e9be16b68a2561c2250cc27`
passed the native qualification on November 23, 2026. The setup verified
the compiled deadline-repair marker in the fresh runtime before linking.
Both ELFs used the frozen V1 broker and application fixture. Four native
diagnostic arms and 16 native fault cases passed on PostgreSQL 17.11.
[Qualification](evidence/ordinary-trusted-product-db5937e7-ddd7f9d0e2e1/qualification.json)

| Native runner | ELF SHA-256 |
| --- | --- |
| Product `db5937e7` | `6a83817cf1992b36e77515afb70a9d8e8cdeb75034f40600bfa17c213c124b8e` |
| Frozen ordinary V1 | `faefaf34344958650a0fbdd19556d86d9f5126c5007b9c77225447004a6781e4` |

One paired timing block per profile used CPUs 0 and 1 for PostgreSQL and
workers, with the same input and broker. Throughput is completed jobs per
second through final durable settlement.

| Profile | Product from submit | Frozen V1 from submit | Product from first commit | Frozen V1 from first commit |
| --- | ---: | ---: | ---: | ---: |
| Short, 128 jobs | 456.34 | 383.80 | 614.56 | 509.53 |
| Dense, 1,024 jobs | 366.31 | 359.95 | 462.60 | 448.68 |

The packet preserves the emitted qualification inventory, its normalized
host-field derivative used by timing, and the raw proof and comparison.
These are single-block transfer results for this exact product ELF.
[Comparison and raw hashes](evidence/ordinary-product-v1-short-dense/summary-partial.json)

```sh
suite=tools/durable-jobs-bench
packet="$suite/results/2026-11-23-ordinary-trusted-product-db5937e7"
PYTHONPATH="$suite" python3 -B "$packet/replay/interleave.py" summarize ordinary-product-v1-short-dense
```

The replay reports unrun crossover groups with exit status 1. Its two
selected blocks match the archived partial summary.
