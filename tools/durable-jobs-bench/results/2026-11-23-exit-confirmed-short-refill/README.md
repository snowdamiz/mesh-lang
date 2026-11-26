# Exit-confirmed short refill on local PostgreSQL

**The refill reduced claims and missed the elapsed-time gate.** In six opposite-order blocks, the three-slot, 500 µs policy saved a median 7.75 claims per 128-job arm versus the paired zero-hold controls. Median submit-to-terminal gain was 2.94 ms and enqueue-commit-to-terminal was 0.78 ms slower. Both clocks beat both zero-hold duplicates in one of six groups. The experiment stays local. [Preselection](PRESELECTION.md) · [Per-arm and paired timing](timing-summary.json) · [Portable receipts](local-evidence.tar.gz)

## Untimed baseline probe

The real zero-hold dispatcher reserved one or two slots on 16 of 34 successful post-initial turns. All successful claims filled their reservation, and every successful claim through the last entry remained within the 16-slot capacity. More slots became confirmed free while 22 of the 34 refill requests were in flight. Eleven small turns could have requested at least three using the confirmed capacity visible by reply, with six seeing it within 500 µs at the probe's sampled boundaries. One empty turn landed after the final successful claim but before the last entry. Four more empty turns followed through shutdown. There were zero claim errors or contention hints in that window. [Probe observations](probe-summary.json) · [Probe patch](untimed-probe.patch)

Median request-to-session, session duration and session-to-dispatch were 18 µs, 1,251 µs and 16 µs. Refill claim sessions summed to 45.3 ms, concurrent with the rest of the workload. The changed-state snapshots averaged 0.45 unassigned slots between enqueue commit and last entry. They observed 45.7 slot-ms of free capacity outside an in-flight reservation and 7.4 slot-ms with no reservation. The snapshots can miss releases obscured by a launch in the same pass. On the probed arm the small batches reflected confirmed capacity at the reservation point, and most of the measured round trip was in the claim session. [Probe reducer](probe.py)

## Policy and controls

The patch applies to the archived r2 runner under an explicit local switch. It waits only on the single split-claim queue with the zero-hold success writer, following a full grant, and while another handler can still make progress. The first eligible sample starts one absolute deadline. A third confirmed free slot or the deadline makes the queue eligible. Queue notifications, cold start, partial or empty grants, and no remaining handler progress take the existing path. Lifecycle work proceeds on its original schedule, including stop, renewal and settlement handling. The existing reservation still caps local capacity, and existing SQL and confirmed-commit checks still gate actor launch. [Overlay](refill.patch)

Every timed arm used the same release-compiled runner. The five arms per block were writer-off A and B, zero-hold A and B, and zero-hold plus refill. The native producer and migrator for this local run were the exact combined local broker artifact. The archived handler, source for each write and C helpers match byte-for-byte. The rebuild used a different local compiler executable from the previous local combined receipt. In the four handler/write symbols inspected, instruction counts matched and differences were confined to relocations and the embedded diagnostic source-path length. Both embedded INSERT statements matched byte-for-byte. [Handler and SQL identity](handler-identity.json)

All 30 ledgers reconciled 128 jobs, one producer transaction and 256 independent application commits each. The fixture confirmed one runner with 16 slots and two scheduler workers, eight warm application connections, 13 peak physical connections, and PostgreSQL 17.11 with verified TLS, the cap of 20, fsync, full-page writes, synchronous commit and commit timestamps. The timed windows used the shared local lock and private PostgreSQL clusters. The Darwin arm64 host had 15 logical CPUs. The fixture's sampled peak one-minute load ranged from 15.8 to 29.8, with wide duplicate spreads. There were no failed timed arms. [Runner and database receipts](local-evidence.tar.gz)

## Timing and Linux gap

Positive values in the first two columns mean the candidate was earlier. Gains subtract the refill time from the same block's zero-hold midpoint. Duplicate spreads are the larger of the median absolute zero-hold A/B spread and the median absolute writer-off A/B spread. The Linux column gives the historical median zero-hold deficit to Apalis in the five matched Linux groups.

| Milestone | Local median gain, ms | Local paired range, ms | Local duplicate spread, ms | Linux deficit, ms |
| --- | ---: | ---: | ---: | ---: |
| Submit to terminal | +2.94 | −86.78 to +32.53 | 35.12 | 66.16 |
| Enqueue commit to terminal | −0.78 | −70.68 to +29.55 | 40.18 | 36.52 |
| Last entry | +0.92 | −69.95 to +28.33 | 39.64 | 37.84 |
| Last first write | +0.47 | −70.27 to +29.52 | 40.09 | 40.89 |
| Last second write | −0.78 | −70.74 to +29.71 | 40.12 | 39.94 |

The candidate beat both duplicates on claim count in five of six blocks. Counts were 37–45 with refill, 40–52 across zero-hold controls and 14–34 across writer-off controls. The median paired reduction in summed PostgreSQL admitted-statement execution, including idle queries, was only 1.63 ms. Rank 64 to rank 128 advanced 3.80 ms in the median paired result and beat both zero-hold duplicates twice. Median terminal tail grew by 0.18 ms. Runner CPU, measured across the whole process including startup and idle, grew by 1.48 ms and sampled peak RSS by 52 KiB. The candidate kept 13 peak connections and 10 sampled threads. No secondary metric exceeded its preselected tolerance. The fastest and slowest refill commit-to-terminal times were 39.9 and 134.3 ms. Across the zero duplicates they were 48.3 and 161.7 ms. [Timing reducer and data](campaign.py) · [Linux matched-gap analysis](../2026-11-23-exit-confirmed-short-gap/README.md)

The local screen establishes that fewer claim turns alone did not reliably move the last entry, application commits or terminal commit. The observed progress gain is small next to the historical Linux shortfall and within local duplicate spread. This experiment covers the exit-confirmed short queue. The earlier fixed-width32 and D2 screens use separate policies and evidence.

## Correctness gates

The workspace build, refill-deadline unit test and scheduler stack-cleanup test passed. The PostgreSQL Jobs gate passed 38 protocol, 73 Jobs runtime and three compiled end-to-end tests. On the enabled 16-slot runner, three sparse submissions into an idle queue entered in 2.06–7.81 ms after enqueue commit. The teardown witness used the same release runtime as timing. The job stayed executing while the actor's termination callback was held and completed after release with the original application XIDs.

The initial locked-middle fault replay held the priming completion receipt for 3.03 seconds. The peers' 1.5-second leases expired before the receipt was released, and the first-attempt assertion timed out. The unchanged case passed on retry with refill enabled and in the same-binary zero-hold control. All seven candidate fault cases passed on the isolated retry, covering the locked member, stop, ambiguous claim and completion receipts, lease expiry, renewal priority and cancellation. The initial failure and subsequent receipts are retained. [Verification index](verification.json) · [Full logs and fault receipts](local-evidence.tar.gz)

## Replay the sealed local measurements

From the repository root, extract the portable receipts into task-private scratch and recompute both summaries. The broker path in the receipts is relative to the checkout. The archive contains no executables, database directories or TLS keys.

```sh
r=tools/durable-jobs-bench/results/2026-11-23-exit-confirmed-short-refill
lane=target/exit-confirmed-short-refill
mkdir -p "$lane/replay"
(cd "$r" && shasum -a 256 -c SHA256SUMS)
tar xzf "$r/local-evidence.tar.gz" -C "$lane/replay"
python3 -B "$r/campaign.py" summarize "$lane/replay/timing" | cmp - "$r/timing-summary.json"
python3 -B "$r/probe.py" "$lane/replay/probe" | cmp - "$r/probe-summary.json"
```

A fresh build starts with the `combined-source.tar.gz` recorded by the original combined study. Apply `untimed-probe.patch` and `refill.patch` to separate clean extracts, using only the probe build for the untimed observation. Use Rust 1.99.0, LLVM 21 and `cargo build --locked --offline --release -p mesh-rt` with a private target. Link the unchanged fixture through the archived `source/replay/link.sh` with the patch SHA as `COMPLETION_WRITER_SOURCE_SHA256`. `campaign.py` uses the archived combined Python adapter, exact local bulk broker and PostgreSQL 17.11. The `probe` and `timing` commands accept a new evidence path and the corresponding `--runner` artifact directory. [Combined source and broker provenance](../2026-11-23-ordinary-completion-writer/combined/README.md)
