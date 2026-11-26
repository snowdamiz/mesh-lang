# Dense claim activity on the two-CPU Linode

Both traced two-runner arms fetched more than 146,000 heap tuples through indexes on the jobs table for 1,024 completed jobs. Each arm granted at most 16 jobs per claim. PostgreSQL recorded zero rows read by sequential scans on that table. The relation counters cover all statements in the cluster. They do not identify which plan node fetched the tuples.

| Deployment and order | Claim calls during submit-to-observation | Jobs table index tuples fetched | Queue backend CPU in observation bracket | Drain change with trace vs duplicate mean |
|---|---:|---:|---:|---:|
| 2 × 32, forward | 104 | 147,218 | 1,219 ms | +54 ms |
| 2 × 32, reverse | 105 | 146,252 | 1,220 ms | −28 ms |
| 8 × 8, forward | 272 | 376,288 | 1,857 ms | +104 ms |
| 8 × 8, reverse | 267 | 395,933 | 2,107 ms | +627 ms |

At 2 × 32 the nested statement with query ID `6827047345048828466` recorded 131,661 and 132,080 shared buffer hits. Its SQL text was redacted in the exports. The identifiable queue-row lock statement accumulated 573 and 516 ms across overlapping callers. At 8 × 8 it accumulated 12.76 and 16.53 seconds. The two-CPU host, public ELF, PostgreSQL build, and frozen input matched across all 12 arms.

The plan probe found no auto_explain module at its configured path. To evaluate a claim rewrite, inspect actual rows and loops for the ordered selection and the subsequent update against the locked prefix on this build. The per-statement elapsed sums include waits, and the queue-role CPU includes settlement and other commands. Neither supports attributing the measured drain gap to a single join.

From this directory, `python3 -B controller.py replay` verifies the 12 raw ledgers and reconstructs the [two rotations](evidence/native/final-replay.json). [Controller release](evidence/native/controller-release.json)
