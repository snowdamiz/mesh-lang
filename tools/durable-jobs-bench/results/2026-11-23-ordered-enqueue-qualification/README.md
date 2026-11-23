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

The AMD64 producer qualified for the matrix owner's five-group matched
repeat after worker attribution. The 1,024-job batch committed in 403.82
ms at the median, compared with 453.38 and 453.74 ms for stock. The
Linux truncated-response unit test remains open.
[Native results](NATIVE.md) · [Inventory handoff](READY.json)
· [Native baseline](../2026-11-23-baseline/README.md)

An isolated integration check on product base `ba8dab03` passed the
workspace and PostgreSQL proofs with the ordered-enqueue patch applied.
The isolated native build still uses the sealed ordered-enqueue source.
[Integration verification](integration-verification.json)
