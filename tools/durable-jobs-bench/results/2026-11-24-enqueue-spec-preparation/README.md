# Enqueue specification preparation

This packet screens an isolated runtime patch on the existing Linux AMD64 Linode. The candidate moves the validated args and metadata trees into the canonical object and uses serde's compact byte serializer. The production decoder, validation order, uniqueness and replacement policies, map ordering, SHA-256, caps, and SQL stay on their existing paths. Test context and standalone validation use the original preparation path.

The component attributes stock time to untagged decode, validation and policy normalization, conversion to a JSON value, serialization, and SHA-256. A token-dispatched rewrite needs its own decoder proof. The stock decoder accepts a positional single JobSpec encoded as an array, as well as batches of objects. An array-to-batch dispatch would change that behavior.

## Native decision scale

| Retained Linode series | Stock submit to commit | Same-rotation headroom |
| --- | --- | --- |
| Earlier Bulk+D2 short | 32.7–37.2 ms | River won three of five submit windows. Closing those gaps with drain fixed required 18.1, 29.8, and 19.1 ms. |
| Six-lane Bulk+D2 short | 34.0–49.2 ms | Bulk+D2 beat River and Oban in five of five submit windows. Apalis led by 53.1–74.8 ms, more than the entire enqueue phase in each rotation. |
| Direct-file large stock A and B | 732.7–793.1 ms | The duplicate mean led Oban and River in all five submit windows. Apalis required 439.6–528.1 ms of savings with drain fixed. Max stock-duplicate enqueue spread was 26.2 ms. |

The retained one-pass large broker changed the Mesh collector upstream of this experiment. It produced a 0.09 jobs/s paired median submit gain against the duplicate mean, inside the 2.59 jobs/s maximum duplicate spread. The older preparation prototype ran on Darwin ARM64. Its roughly 0.08 ms short saving and overlapping large results are historical context. It also changed decode routing and did not prove malformed-input compatibility. [Native paired budgets](budgets.json) · [One-pass broker](../2026-11-23-ordinary-broker-once/README.md) · [Historical preparation prototype](../2026-11-23-spec-encoding/README.md)

Advance a profile only when its median CPU saving against the faster stock control reaches 5 ms per 128-spec short batch or 40 ms per 64-job large submission, and the candidate beats both the CPU spread and the faster control's wall time in at least five of six rotations. Those thresholds represent roughly 14% and 5% of observed enqueue latency at their respective medians. The component excludes TLS, PostgreSQL, producer IPC, and worker execution. A qualifying result triggers a durable broker trial using those paths.

## Exact inputs and replay

The short fixture uses the measured Bulk+D2 broker's session transport, collector and encoder. It emits one 154,241-byte frame with 128 specs. The large fixture uses the ordinary direct-file broker's collector and encoder. It emits 64 frames of 524,470 bytes each, in order. Untimed compiled collectors extracted those frames from the original seeded producer fixtures and verified every payload against the corresponding measured durable ledger. Native SETUP must recapture all frames using the resident pinned compiler and match all frame hashes before building the components. [Short capture](short-capture.json) · [Large capture](large-capture.json)

The component uses the archived Bulk V2 runtime for short and the stock runtime at 3828f20d2d6beb683868cce33e6dd1958365914b for large. Cargo output directories are independent. SETUP rejects identical short and large binaries before probing PostgreSQL. Release builds use the frozen lockfiles, Rust 1.99, the production profile, and default target flags. Stock A and B run the same executable with the same inputs in fresh processes. Timed component arms pin CPU 0. The compiled broker trial uses the original CPUs 0 and 1 and pinned PostgreSQL 17.11 settings.

Local functional runs use Darwin ARM64. The semantic probe compares canonical bytes, request keys, fingerprints, accepted shapes and errors across 59 cases plus all real frames. The wire proof uses an owned PostgreSQL 17.11 cluster and both enqueue implementations with durable job and receipt counts, a changed-spec conflict, savepoint recovery, and outer rollback. It requires the set-based entry point to execute for short and retains separate writer counts for both profiles. The native durability scorer checks one enqueue transaction, 128 or 64 matching durable IDs, and two independent application commits per job. [Local evidence](evidence/local/) · [Retained failures](previous-failures/)

Run offline checks from the repository root.

    d=tools/durable-jobs-bench/results/2026-11-24-enqueue-spec-preparation
    python3 -B "$d/offline.py"
    python3 -B "$d/controller.py" check
    (cd "$d" && shasum -a 256 -c SHA256SUMS)

The matrix controller owns the host and grants each phase separately. The request records the commands and receipt destinations. SETUP builds, hashes, and exports both AMD64 executables and untimed proofs. Component TIMING returns raw per-arm rows and a platform witness. For a qualifying profile, a new SETUP links the production-only patch into the pinned stock Mesh broker source, using the original native library bindings. The trial inventory carries two duplicate controls, the candidate, and the original Oban, River, and Apalis arms. The runner, migrator, pool and completion options, workload, and clocks use the frozen reference. The original matrix and the durable broker scorer replay five matched rotations, including the profile's normal warmups. The summary reports full-submit wall time from the first producer submit through the last durable settlement as the primary clock. Submit to enqueue commit is secondary. The verdict requires a median saving of at least 5 ms short or 40 ms large against the faster duplicate and a win exceeding the within-rotation duplicate spread in at least four of five rotations. It also includes the paired margins against Oban, River and Apalis.

The archive includes the two patched production store.rs files and their source hashes in candidate-identities.json. The controller rebuilds the producer from the matching stock source tree and verifies the production patch and resulting store.rs before linking. The exact link receipt is required for full-submit dispatch.

    python3 -B "$d/rotation.py" replay "$d/evidence/timing-<suffix>/raw"

No performance samples were collected during implementation. A Linode component result and any conditional broker result are pending controller assignments.
