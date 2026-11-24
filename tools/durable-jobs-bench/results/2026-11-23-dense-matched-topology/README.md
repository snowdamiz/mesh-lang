# Dense durable Jobs with two native runners

The prospective comparison uses the frozen 1,024-job `noop` batch on Linux
AMD64. It includes two independent series of five rotated blocks. The stock
series compares ordinary V1 Mesh at 2 × 32 with its original 8 × 8 deployment
and the original 8 × 8 Oban, River, and Apalis deployments. The equal-topology
series runs each of the three peers at 2 × 32, alongside both Mesh layouts.
Both series use the same pinned boot, kernel, disk and CPU IDs 0 and 1 for
PostgreSQL and the adapter tree. Each block includes a full unmeasured warmup
arm per lane before its measured arms. Each arm provisions a fresh PostgreSQL
17.11 cluster with verified TLS, fsync, synchronous commit, commit timestamps,
and a physical connection cap of 96. [Stock comparison](stock.json) ·
[Equal-topology comparison](equal.json) · [Published V1 baseline](../2026-11-23-ordinary-trusted-v1/README.md)

| Engine | 8 × 8 runner pool cap | 2 × 32 runner pool cap | Producer start event |
| --- | ---: | ---: | --- |
| Mesh ordinary V1 | 32 queue + 32 app | 8 queue + 32 app | Before reading and sending the batch to the warm broker |
| Oban 2.24.2 | 80 shared | 68 shared | Before sending the batch path to the warm broker |
| River 0.51.2 | 80 shared | 68 shared | Before reading the batch file |
| Apalis 1.2.0 | 80 shared | 68 shared | Before sending the batch to the warm producer session |

Each deployment has 64 aggregate worker slots. Mesh, Oban, and Apalis
configure two scheduler or Tokio workers per runner. River is restricted to
the same two CPU IDs and uses GOMAXPROCS 2 for the producer/observer. The runner pool caps omit the dedicated producer/observer
connection. Mesh reserves, warms, and verifies 32 distinct application
sessions under its scoped application role. The peer adapters use shared
runner pools for queue and application writes. Replay records physical warm
sessions where the native adapter exposes them and reports the observed
connection peak separately from configured capacity. Live BEAM processes,
River worker reports, Apalis listener readiness, and Mesh runner reports
establish the actual process topology. The peer launch overlays bind both
the runtime and its evidence to two 32-slot runner processes. River's stock
binary rebuilt byte-for-byte before the only change, the dense runner-count
literal. The 2 × 32 Oban and Apalis lanes reuse the existing binaries.
[Peer build receipt](river-build.json) · [Stock qualification](offline-stock.json) ·
[Equal-topology local functional checks](offline-peers.json)

The submit score counts completed jobs from the actual producer event through
the final durable terminal commit. The drain score starts at the producer
transaction's PostgreSQL commit and ends at the same terminal commit. Both
scores include live consumption and the same two independently committed
application writes per job. The verifier checks each frozen payload, producer
receipt, job ID, terminal state and work XID. An explicit diagnostic runner
count is bound to an isolated verifier while the ledger keeps the real count.
The original eight-runner verifier and canonical results retain their
identities. Lane order rotates through all five ordinal positions. Blocks
with errors, incomplete evidence, builders, or more than 1% measured CPU
steal are excluded as whole blocks. Their receipts remain available.
[Dispatcher and replay](matrix.py) · [Qualification](qualification.py)

## Native stock-peer comparison

All five rotations qualified. The medians for Mesh V1 2 × 32 were
547.01 jobs/s from submit and 751.82 jobs/s from the first enqueue commit.
Mesh V1 8 × 8 reached 362.63 and 452.45 jobs/s. The paired median increase
was 50.8% from submit and 66.2% from commit. Mesh 2 × 32 beat Oban 8 × 8
on drain in all five rotations, with a paired median lead of 8.1%. Oban
led from submit in all five, and River and Apalis led in both windows in
every rotation. The scorecard reports each lane's median and full range,
observed connections, runner CPU, and terminal transaction count.
[Stock-peer scorecard](evidence/stock/scorecard.md) ·
[Qualification replay](evidence/stock/replay.json) ·
[Controller release](evidence/stock/controller-release.json)

The two-runner layout warmed the same 32 Mesh application sessions and
used four distinct backends for those writes in each measured arm. The
eight-runner layout used 14–16. Two-runner median runner CPU fell from
1.615 to 0.637 ms/job. The serial settlement contract continued to use
1,024 terminal XIDs per Mesh arm. River and Apalis used their stock
settlement batching, whose transaction counts are shown separately.
[Stock-peer scorecard](evidence/stock/scorecard.md)

## Native equal-topology interruption

The peer setup released successfully with pinned launcher and binary hashes.
All five first-block warmup commands exited 0. Four raw warmup archives
reconciled and confirm native Mesh, River, and Apalis launch topology. The
Oban warmup export timed out. The measured dispatches then failed to connect
to SSH on port 22. The controller placed the series on hold pending
recovery of the existing route and verification of the host and locks.
Zero equal-topology blocks qualified. The original block and all receipts
are retained. [Peer setup receipt](evidence/peer-setup/setup.json) ·
[Raw warmup replay](evidence/equal/warmup-only-replay.json) ·
[Interrupted block receipt](evidence/equal/interrupted-01.json) ·
[Controller hold and retained staging](evidence/controller-hold.json)

## Producer-only composition

The bounded follow-up swaps only the broker executable for the pinned
set-based producer. Both arms run the exact same original V1 2 × 32 consumers
and install the same additive bulk-capable schema before the measured window.
The local functional smokes each completed 1,024 jobs and validated 3,073
distinct producer, application, and terminal XIDs. Both made 4,096 scoped
application SQL calls. The baseline issued 1,024 scalar enqueues. The
set-based producer issued eight bulk calls and zero scalar enqueues.
The historical fixed-drain model combines separately observed timing
windows and serves only as a comparison envelope. Two opposite-order native
pairs require their own controller grant and do not delay either five-block
series. [Local checks and historical envelope](offline-bulk.json) ·
[Producer-only request](request-bulk.json)

## Replay

Run from the checkout root. The historical checks use archived receipts and
need no database or remote access.

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-dense-matched-topology
python3 -B "$packet/offline.py"
python3 -B "$packet/replay-local.py"
python3 -B "$packet/matrix.py" check stock
python3 -B "$packet/matrix.py" check equal
python3 -B "$packet/bulk.py" check
python3 -B "$packet/matrix.py" replay stock --directory "$packet/evidence/stock"
```

The existing native controller owns SETUP and TIMING. Request SHA-256 pins
bind the local scripts and inventories. Each native worker verifies the
pinned artifacts on the host, records the CPU mask, rejects other builders,
and exports redacted raw evidence under the established lock sequence.
[Stock timing request](request-stock.json) · [Peer setup request](request-peer-setup.json) ·
[Equal-topology timing request](request-equal.json)

The local workspace suite passed 4,821 tests. The PostgreSQL gate passed the
scoped role proofs on fresh and upgraded databases and the compiled Jobs
end-to-end tests. [Workspace result](evidence/gates/workspace.json) ·
[PostgreSQL proof](evidence/gates/postgres-proof.json)
