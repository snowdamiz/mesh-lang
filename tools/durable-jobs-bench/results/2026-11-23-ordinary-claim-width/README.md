# Separating runner count from claim width on a dense queue

In the two native Linux rotations the two-runner deployment completed its final claim about a second earlier. The faster eight-runner control still had 445 and 447–448 jobs unclaimed when two runners had already settled all 1,024. A claimed job actually waited longer on average for handler entry with two runners. The improvement came alongside more overlap and faster sustained admission. [Native progress and slot residence](native-progress.md)

The local follow-up kept the compiled V1 executable, producer, two independently committed application writes per job, PostgreSQL 17.11, and the 64-slot global cap fixed. It ran two Mesh runners at 32 slots each and changed only the installed claim function’s per-call grant limit from 16 to eight. Two duplicate controls kept 16. A fourth arm used the original eight-runner deployment at the same effective maximum of eight grants. The two-runner configurations retained 32 warm application sessions and eight lifecycle sessions. Eight runners retained 32 of each.

| Deployment | Drain jobs/s | Runner CPU ms/job | Claim calls per 1,024 jobs | Mean occupied slots |
|---|---:|---:|---:|---:|
| Eight runners, width 8 | 1,984 | 1.672 | 486 | 15.44 |
| Two runners, width 8 | 1,503 | 0.983 | 198.5 | 19.06 |
| Two runners, width 16 A | 2,123 | 0.914 | 149 | 31.90 |
| Two runners, width 16 B | 2,643 | 0.848 | 153 | 33.07 |

Values are medians of four on-host rotations. At equal width eight, reducing process count saved **39–46% runner CPU/job in every rotation**. The two-runner arm drained faster just once at that width. At fixed two-runner topology, width eight used more runner CPU than both width-16 controls in every rotation and drained slower than both in three. Reducing process count explained a CPU gain independently of the wider claims it allowed. The observed rate also depended on claim width. Each local arm completed 1,024 single attempts with 2,048 distinct application transactions and 1,024 distinct terminal transactions. [Per-rotation scores](local-results.json)

These are macOS ARM64 development-linked runs on a shared host with load averages of 26–37 at arm start. The workload used verified PostgreSQL TLS. The exact SQL difference is a smaller upper bound on grants in the cursor function in a fresh fixture database. The full reply byte limits, transaction boundaries, pauses and capacity settings came from the frozen binary and schema. The reducer checks both function definition hashes, the live connection census, per-runner grant totals, receipt ordering and all XIDs. The native deployment gain has its own separate two-CPU evidence.

## Replay

From the repository root, replay all 16 compiled local arms and regenerate the native decomposition.

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-ordinary-claim-width
python3 -B "$packet/results.py"
python3 -B "$packet/topology.py"
```

A new local run uses the retained development executable, PostgreSQL 17.11 and the same V1 source and artifact pins as the original topology experiment. The launcher applies the grant limit before the broker starts and reads the installed SQL back from PostgreSQL. It takes the local queue-performance lock. [Pinned build and prior live replay](../2026-11-23-ordinary-runner-topology/README.md)

```sh
mkdir -p target/ordinary-claim-width
python3 -B "$packet/screen.py" target/ordinary-claim-width/repeat
```
