# Enqueue specification preparation

The isolated runtime patch reduced specification preparation time on the Linux AMD64 Linode. Both profiles fell below their original producer trial thresholds. The candidate moves the validated args and metadata trees into the canonical object and uses serde's compact byte serializer. The production decoder, validation order, uniqueness and replacement policies, map ordering, SHA-256, caps, and SQL stay on their existing paths. Test context and standalone validation use the original preparation path.

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

## Released Linode result

The original SETUP exited before creating its staging directory. The repair switched findmnt to raw output and retained the exact block device match on 8:0. The repaired SETUP passed on Linux AMD64 with PostgreSQL 17.11, including 32 old/new replay cases, savepoint recovery and outer rollback for each profile. The controller released all six timing rotations at 04:24:44 UTC. [Original failure](evidence/setup-de98a0588ecd/controller-release.json) · [Mount repair and successful setup](request-mount-retry.json) · [Timing release](evidence/timing-be34d253e19e/controller-release.json) · [Raw timing archive](evidence/timing-be34d253e19e/raw.tar.gz)

| Profile | Input to preparation | CPU saving against faster stock, median [range] | Wall saving against faster stock, median [range] | Original CPU threshold |
| --- | --- | --- | --- | --- |
| Short | 128 jobs with 1 KiB payloads, one 154,241-byte UTF-8 frame (150.63 KiB) | 0.177 [0.165–0.184] ms | 0.178 [0.164–0.185] ms | 5 ms |
| Large | 64 jobs with 512 KiB payloads, 64 frames of 524,470 bytes, totaling 33,566,080 bytes (32.01 MiB) | 34.542 [31.645–35.126] ms | 34.536 [31.655–35.127] ms | 40 ms |

The payloads total 128 KiB and 32 MiB before job fields and framing. Each saving is per complete submission. Both profiles beat stock duplicate jitter and the faster stock wall time in six of six rotations. The frozen scorer replayed all 48 arm receipts, including 12 diagnostic stock-stage receipts, and reproduced both decisions. Neither profile advanced to broker linking or full-submit validation. [Frozen summary](evidence/timing-be34d253e19e/raw/summary.json) · [Independent replay receipt](evidence/verified-result.json)

| Diagnostic stock stage | Short median, ms/submission | Large median, ms/submission |
| --- | ---: | ---: |
| Untagged decode | 0.201 | 24.444 |
| Validation and policy normalization | 0.010 | 0.017 |
| Conversion to a JSON value | 0.204 | 24.669 |
| Serialization | 0.383 | 48.911 |
| SHA-256 | 0.140 | 22.740 |

Process CPU and monotonic wall bracket only the repeated preparation loop and disposal of its outputs, after loading the frames and before the output-digest check. Whole-stock and candidate arms have no per-stage instrumentation. Stage times are instrumented stock wall medians. Short averages 96 repetitions of its one-frame submission per process, and large averages eight repetitions of its 64-frame submission. The component excludes fixture I/O, broker transport, TLS, database work and worker completion.

## Share of the historical producer gap

> These are scenario estimates. Apply the measured median component wall saving unchanged to each historical Linode rotation and hold the rest of the arm fixed. The submit-to-commit clock ends at durable enqueue commit. The full-submit clock extends to the last durable settlement.

- Short accounted for **0.36–0.54%** of historical stock submit-to-commit time. It could cover **0.60–0.98%** of the three earlier full-submit losses to River and **0.24–0.34%** of the later full-submit gap to Apalis. In the six-lane series there was no full-submit gap to River or Oban, even though River's submit-to-commit time was 17.5–31.9 ms shorter. The component could cover **0.56–1.02%** of that producer gap.
- Large accounted for **4.43–4.67%** of historical stock submit-to-commit time. It could cover **27.1–36.8%** of Apalis's 93.8–127.4 ms submit-to-commit lead and **6.54–7.86%** of its 439.6–528.1 ms full-submit lead. River and Oban already trailed Mesh stock in both clocks in all five large rotations.

The archived clocks do not establish how much of the component saving would transfer to a real broker. [Per-rotation arithmetic and receipt hashes](evidence/verified-result.json)

## Offline replay

From any checkout containing the archive and report, use Python 3.11 or later. The replay reads Git-backed files using relative paths and does not launch a broker, run a benchmark, or contact the host.

    d=tools/durable-jobs-bench/results/2026-11-24-enqueue-spec-preparation
    python3 -B "$d/replay-result.py" | cmp "$d/evidence/verified-result.json" -

The original failed export and mount repair were committed in 20c257ee. The controller archived the byte-exact native setup and timing receipts in f8b0bf40. This replay reuses those files and the frozen scorer without rewriting the raw evidence. The original submission packet and its input checksum list remain historical witnesses, including the README hash from before this report.
