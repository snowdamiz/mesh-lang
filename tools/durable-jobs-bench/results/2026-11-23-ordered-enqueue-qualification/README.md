# Ordered enqueue native qualification

Candidate `77c447f773a4d0af5c5578873a2410301b3ba069` batches PostgreSQL
writes for Jobs enqueue while retaining a separate SQL statement and Sync
for each admission. Its source is sealed against Mesh stock
`3f69b150820fd01ba716d533be4f2eedea25ed27`.
[Candidate patch](candidate.patch.gz) · [Source seals](seals.json)

On Darwin arm64, the 128-job producer committed sooner in all five paired
groups against both stock controls. The dense producer sent 237 client
TLS records, compared with 1,053 in each stock control. Dense durable
commit timing was inconclusive. Sparse and large sent the same number of
records as stock. These measurements cover the producer path through
durable commit. [Original local measurements](original-local-measurements.json)

Fresh verification on the exact candidate passed workspace and
PostgreSQL 17.11 proofs. The compiled Darwin producer passed a SQL error
after three attempted inserts, a backend disconnect at the same point,
and a rollback followed by a commit in the same session. The database
recorded zero durable rows after either injected failure. The first two
local replay attempts failed on diagnostic assertions and are retained
with the passing replay.
[Local verification](local-verification.json) · [Fault receipts](local-faults.tar.gz)

Native AMD64 qualification is pending an exclusive SETUP window from
`linode-queue-matrix-and-iterations-v1`. The official Mesh, Oban, River, and
Apalis baseline sealed all 25 blocks at `069c9c68`. The owner is running
a separate focused comparison for the scheduled-work query candidate
`ee593226` after sealing its 25-block repeat. The worker-cost lane owns
dense attribution and queued its SETUP after ordered enqueue.
[Native baseline](../2026-11-23-baseline/README.md) · [Run the qualification](REPLAY.md)

An isolated integration check on product base `ba8dab03` passed the
workspace and PostgreSQL proofs with the ordered-enqueue patch applied.
The isolated native build still uses the sealed ordered-enqueue source.
[Integration verification](integration-verification.json)
