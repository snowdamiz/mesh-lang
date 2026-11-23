# Protected Jobs admission and settlement experiment

The explicit-transaction candidate implements queue-ordered capacity grants, set-based claims, and an optional cohort of already-reaped successes. Five matched Linux guest runs confirm the lower SQL call count. Host load and wide paired ranges kept the guest throughput result inconclusive. The first independent AMD64 rotation shows a smaller gain for strict admission and leaves a substantial peer gap. Strict mode preserves a separate terminal XID for every job.

The isolated source starts at [BASE](BASE). The complete patch is [admission-v3.patch](admission-v3.patch), with the local commit recorded in [CANDIDATE](CANDIDATE). The released dispatcher and SQL are selectable as `stock`. Candidate selectors are `MESH_JOBS_ADMISSION_V3=strict` and `MESH_JOBS_ADMISSION_V3=cohort` for existing batch runners. The protected application sessions, handler entry, both independent writes, fenced reset acknowledgement, fresh process execution, process ownership, and consuming wait are exercised through the existing fixture.

## Transaction and dispatch protocol

- Each runner reserves slots against its actual local queue and runner limits before requesting a grant. PostgreSQL's per-namespace/queue row lock orders those requests, then the candidate reads executing occupancy and acquires the existing namespace pause fence. The queue and lease rows remain the durable authority shared by v1 and v3. There is one pending request per runner. A bounded lock wait replaces optimistic contention loops for the ordinary v3 path.
- One set-based update claims the ordered prefix of currently eligible, lockable jobs for the permitted workers. It checks actual wire sizes and bounds speculative JSON rendering. Large or compressed values, a different client encoding, and ambiguous emptiness due to locked eligible job rows use the released exact-prefix batch claim. A definitive empty turn carries its scan cutoff so a scheduled time that crosses before the due probe is still visible.
- Admission starts a short READ COMMITTED transaction. The client verifies the entire row stream against PostgreSQL's row count and ordinal sequence, checks the context envelope, then requests COMMIT. Launch requires both the COMMIT tag and idle ReadyForQuery. An invalid or partial grant response rolls back, including when only one attempt is allowed. A lost COMMIT response suppresses launch and leaves recovery to the durable lease.
- Strict mode uses the released scalar settlement transactions. Cohort is a separate product option. A single transaction completes up to eight immediately available successful outcomes after exact consuming waits and completed effect relays. This is one shared terminal XID for that cohort. Busy, stale, cancelled, expired, exceptional, and singleton cases retain the scalar path. A cohort rejection rolls the transaction back, after which its original attempts can settle independently. On an ambiguous receipt, the runtime delays and retries the original tokens individually. It publishes neither successful completion nor refill capacity until acknowledgement. Claim I/O retains the existing separate lane when the configured topology supports it.
- Optional v3 grants live in a separate ACL file and apply after candidate migration. The ordinary grants fixture continues to install against v1. Bounded drains and nonbatch runners keep the released dispatch path.

## PostgreSQL operation pricing

Five fresh verified-TLS PostgreSQL 17.11 arms per mode used the frozen 1,024-job input, a real committed product row and enqueue transaction, durable configuration, and real writes under scoped roles. The sequential libpq driver made turns of eight, with 1,024 independent first-write commits, 1,024 independent second-write commits, and 2,048 acknowledged `DISCARD ALL` resets per arm. Backend CPU comes from OS process counters around the real exchanges. The test has one admission connection and one protected application connection, with no live runner contention. It measures statement and transaction work on this local host, while the guest and native arms exercise dispatch.

| Median backend CPU | Stock | Strict | Cohort |
| --- | ---: | ---: | ---: |
| Admission, ms/job | 0.222 | 0.147 | 0.142 |
| Due probe, ms/job | 0.050 | 0 | 0 |
| Settlement, ms/job | 0.270 | 0.279 | 0.085 |
| Admission + probe + settlement, ms/job | 0.537 | 0.426 | 0.226 |
| Range of that total, ms/job | 0.522–0.611 | 0.391–0.466 | 0.215–0.274 |
| Independent first app transaction and reset, ms/job | 0.505 | 0.514 | 0.497 |
| Independent second app transaction and reset, ms/job | 0.460 | 0.459 | 0.461 |
| Both resets, ms/job, included above | 0.077 | 0.076 | 0.077 |

Stock admission costs 1.568 ms per eight-job query, 0.041 ms per BEGIN, and 0.155 ms per COMMIT at the median. Its due probe costs 0.404 ms per turn. The separate 64-operation renewal exercise on active leases costs 0.242 ms per stock renewal. Renewal SQL is unchanged across modes. The due probe is absent on these full candidate turns and still runs when needed after an empty turn. [Raw pricing](receipts/explicit-prices.json) · [Stock-only extraction](receipts/stock.json) · [Reducer result](receipts/summary.json)

## Full-path guest comparison

Each result follows a full-input warmup and a fresh database on the two-vCPU Linux ARM64 guest. Stock, strict, and cohort share one v3 release ELF and choose the dispatch behavior at startup. The pipeline uses the prior real release pipeline ELF and its matching separate fresh worker executable. Its XID semantics are strict. Its differing binary is a confound for attributing differences to any one source edit.

| Mode | Median and range in jobs/s | Median paired rate vs stock | Median runner SQL calls/job | Terminal XIDs |
| --- | ---: | ---: | ---: | ---: |
| Stock | 637, 332–963 | 1.00× | 2.87 | 1,024 |
| Strict | 732, 73–888 | 0.80× | 1.99 | 1,024 |
| Cohort | 946, 559–993 | 1.48× | 1.62 | 658–695 |
| Historical real pipeline | 713, 391–994 | 0.95× | 3.05 | 1,024 |

The cohort-to-stock paired range is 0.61×–2.99×. The stock-to-strict pairs favor strict once in five runs. Host one-minute load at measured-arm start reached 50.1 on 15 logical host CPUs. Each measured arm completed 1,024 jobs with 2,048 independent ordered application XIDs and 2,080 full resets including startup. The maximum sampled connection count was 65 of 96. The maximum sampled executing count was 48 of 64. Separate unscored wait instrumentation tied every successful spawn and consuming wait to PostgreSQL worker and commit identities in both candidate modes. [Per-run results](receipts/summary.json) · [Timing inputs and receipts](receipts/guest-timing.tar.gz) · [Exact-wait receipts](receipts/guest-consuming-waits.tar.gz)

The local 2.38× reduction covers the scripted Jobs admission, due, and settlement CPU slice in cohort mode. Native application work and fresh child lifetimes remain significant independent costs. The local component ratio cannot be applied to the full 3.8-second gap.

## Failed ideas and correctness gates

- The first SQL candidate used an singleton call through the cohort function. In the pricing screen its per-job settlement cost rose relative to stock. Current strict and singleton completion use the stock route. [Initial screen](receipts/initial-singleton-comparison.json)
- The autocommit admission pilot received the durable grant before the caller could validate its payload. A malformed final-attempt result could spend the attempt. Its source and CPU measurements remain diagnostic. [Autocommit patch](autocommit-diagnostic.patch) · [Autocommit price](receipts/autocommit-prices.json)
- On the explicit candidate, real PG17.11 wire faults partially delivered the first final-attempt row, changed the final ordinal, removed the final row, or cut the idle COMMIT receipt. The three precommit faults left both rows available at attempt zero. The postcommit fault exposed no launchable grant. Recovery discarded the exhausted row, made the other retryable, and cleared both tokens. Two further real-wire checks verified that an empty transaction status keeps attempt zero and that substituting a ROLLBACK tag on an already committed response suppresses launch and preserves normal lease recovery. [Wire proof](receipts/wire.json) · [Additional receipt faults](receipts/wire-additional.json)
- Scoped PG17.11 edges exercised v1 and v3 competing for the same queue, eight waiting sessions against three durable slots, priority and version filters, pause/resume, confirmed refill, a locked cohort row, cancellation, lease recovery, and exact large-value fallback. The first SQL screen attempted to compare `xid` as part of a row equality and PostgreSQL rejected it. The implementation uses the individual predicates. [Edge proof](receipts/edges.json) · [Rejected SQL screen](receipts/rejected-row-comparison.log)
- The released effect-fence, PG17.11 compatibility, role and compiled Jobs proofs passed. The initial effect-fence gate detected an installer compatibility issue from adding v3 grants to the shared v1 fixture. The candidate moved the grants into a separate optional fixture and repeated the effect-fence proof. [Effect-fence proof](receipts/verify-fence.log) · [Compatibility and Jobs proofs](receipts/verify-roles.log)
- The alternative in [PACKED](PACKED) sends explicit BEGIN and the grant SELECT in one extended exchange. It waits for transactional ReadyForQuery, validates the complete grant, and issues COMMIT independently. The same four fault cases passed. Two opposite-order full guest screens gave strict-to-stock ratios of 2.02× and 1.29× against widely separated stock controls. This warrants a native falsifier if scheduling permits. [Alternative patch](packed-transport.patch) · [Wire proof](receipts/packed-wire.json) · [Guest results](receipts/packed-timing.json)

## Replay

Run the scripts from the checkout root. The lane is `target/jobs-admission-v3`. Each run gets an isolated PostgreSQL cluster. The pricing and SQL edge scripts require PostgreSQL 17.11, a compatible libpq, OpenSSL and Python 3. Run the native script only through the active matrix-controller grant and existing three locks.

```sh
lane=target/jobs-admission-v3
record=tools/durable-jobs-bench/results/2026-11-23-admission-v3
mkdir -p "$lane"/{stock,source,tmp}
base="$(cat "$record/BASE")"
git archive "$base" | tar -xf - -C "$lane/stock"
git archive "$base" | tar -xf - -C "$lane/source"
(cd "$lane/source" && git apply "$OLDPWD/$record/admission-v3.patch")
cp "$record"/replay/* "$lane/"
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin python3 -B "$lane/price.py" \
  --stock "$lane/stock" --candidate "$lane/source" --scratch "$lane/tmp" \
  --libpq /opt/homebrew/opt/postgresql@17/lib/postgresql/libpq.dylib \
  --arms stock strict cohort --output "$lane/explicit-prices.json"
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin python3 -B "$lane/edges.py" \
  --stock "$lane/stock" --candidate "$lane/source" --scratch "$lane/tmp" \
  --libpq /opt/homebrew/opt/postgresql@17/lib/postgresql/libpq.dylib --output "$lane/edges.json"
```

Build the isolated `mesh-rt` test executable with `cargo test --locked -p mesh-rt --lib --no-run` and pass that executable to `wire_proof.py --candidate "$lane/source" --scratch "$lane/tmp" --output "$lane/wire.json"`. The guest overlay requires the prior frozen Linux guest, fixture object, and real settlement pipeline assets in the target lanes. See the prior [settlement pipeline replay](../../../../target/dense-settlement-pipeline/REPLAY.md). To reduce the committed guest evidence independently, extract the timing archive into a fresh directory and pass its five `committed-*` paths to `summarize.py --runs` with `--price "$record/receipts/explicit-prices.json"`.

## Linode AMD64 setup

The controller built commit `bd7851b9` with the pinned stock compiler. The exact ELF SHA-256 is `cf4f22be4995fd5f7c00a45e1ea97753c1c65577cc34142195f6d98d11087483`. All four setup smokes passed independent ledger replay on CPUs 0 and 1 with verified TLS, PostgreSQL 17.11, durability enabled, and zero reserved connections.

| Profile and mode | Commit-to-terminal jobs/s | Submit-to-terminal jobs/s | Runner CPU-ms/job | Peak connections / cap | Terminal XIDs |
| --- | ---: | ---: | ---: | ---: | ---: |
| No-op strict | 217.05 | 194.51 | 5.514 | 65 / 96 | 1,024 |
| No-op cohort | 217.15 | 193.40 | 5.506 | 65 / 96 | 739 |
| Short strict | 247.20 | 216.93 | 4.794 | 13 / 20 | 128 |
| Short cohort | 275.15 | 239.99 | 4.487 | 13 / 20 | 65 |

Each no-op smoke retained 1,024 distinct worker PIDs, 2,048 independent application commits, and 2,080 full resets including startup. Each short smoke retained 128 distinct worker PIDs, 256 independent application commits, and 264 resets. These are single setup runs. The first independent matched strict rotation follows below. [Reduced smokes](receipts/native-smokes.json) · [Raw native archive](../2026-11-23-admission-v3-setup-bd7851b9/raw.tar.gz) · [Executable pins](receipts/native-artifact.json)

The full isolated workspace test command exited zero with 4,817 passed test entries and 64 ignored across 123 test binaries and doc-test groups. The separately enabled wire and PostgreSQL compatibility checks supplement the default workspace test entry points. [Workspace log](receipts/workspace.log.gz)

Five additional fresh scripted arms per mode at each width priced strict admission, probing and settlement at 0.514 versus stock 0.906 CPU-ms/job for single-job turns, and 0.373 versus 0.595 at width four. These component prices exclude the concurrent dispatcher and do not predict native throughput. [Width one](receipts/price-width-1.json) · [Width four](receipts/price-width-4.json)

## Independent Linode rotation

The controller released the first matched no-op and short blocks at 20:45:34 UTC. Each block compared the exact strict binary with current stock Mesh and the three pinned peers on the same host boot and CPU mask. The table uses committed-enqueue to last-terminal rates.

| Profile | Stock Mesh | Strict admission | Oban | River | Apalis |
| --- | ---: | ---: | ---: | ---: | ---: |
| No-op, jobs/s | 197.67 | 214.23 | 664.84 | 885.97 | 1,237.14 |
| Short, jobs/s | 221.71 | 241.25 | 274.62 | 513.99 | 958.51 |

Strict recorded 188.53 versus 178.37 submit-to-terminal jobs/s for no-op and 211.16 versus 196.55 for short. Runner CPU changed from 5.710 to 5.594 CPU-ms/job for no-op and from 4.787 to 4.668 for short. The measured differences were +8.4% and +8.8% in commit-to-terminal throughput. The strict ledgers again recorded 1,024 and 128 distinct terminal XIDs, the exact protected app commits and resets, and zero cohort calls. The host scorer recorded zero steal in both strict arms. This is one matched pair per profile. The controller rotated to another lane and the remaining rotations are pending. [Independent rotation](receipts/independent-first-rotation.json) · [Series with open blocks](receipts/independent-series-summary.json) · [Controller archive](../2026-11-23-admission-v3-strict/)

The isolated candidate and diagnostic commits are local. Integration and a repeatable throughput verdict remain pending.

The [combined lean-worker source and pins](combined/README.md) are staged independently of admission qualification.
