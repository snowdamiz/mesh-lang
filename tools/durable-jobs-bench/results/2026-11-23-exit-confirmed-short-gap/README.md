# Exit-confirmed short jobs on Linux

**The repeatable deficit is against Apalis.** Both Mesh writer modes enter their first handler sooner than Apalis in all five groups. They fall behind as they refill the 16 slots. Their last entries are about 38 ms later in the median matched group, and their last application commits about 39–40 ms later. Mesh also spends 25–29 ms more in the producer call through its durable commit. Apalis groups terminal updates, but its final drain is as long as or longer than Mesh's in most pairs. The next target is the admission-to-application path during slot refill. [Per-group evidence and source hashes](evidence.json)

## Same-group comparisons

All five six-lane Linux groups contribute. Each arm ran the same 128 short jobs, one producer transaction, one runner with 16 slots, and two separately committed application writes per job. All 30 ledgers reconciled with zero failures. PostgreSQL 17.11 had verified TLS, synchronous commit, fsync, full-page writes, commit timestamps, and a cap of 20 connections. The split Mesh arms used the same runner ELF, bulk broker, and eight warmed application sessions. The older bulk arm is retained in the replay for coverage and context. It uses a different runner and is excluded from the exit-confirmed pairs. [Native series and release verification](../2026-11-23-ordinary-completion-writer-native-repair/evidence/verified-timing.json)

Times below are milliseconds. Each cell is the median **same-group Mesh minus peer** difference, with the five-group range. Positive means Mesh took longer. The two clocks end at the last durable terminal commit.

| Mesh arm | Stock peer | Submit to terminal gap | Enqueue commit to terminal gap | Slower groups, submit / commit |
| --- | --- | ---: | ---: | ---: |
| Writer off | River | +32.3 [−54.4, +46.3] | +9.8 [−80.7, +12.3] | 4/5 · 4/5 |
| Zero hold | River | +26.1 [−53.4, +47.5] | +2.9 [−82.5, +29.6] | 3/5 · 3/5 |
| Writer off | Apalis | +67.4 [+48.4, +135.9] | +38.3 [+24.9, +98.7] | 5/5 · 5/5 |
| Zero hold | Apalis | +66.2 [+47.1, +143.4] | +36.5 [+16.4, +119.1] | 5/5 · 5/5 |
| Writer off | Oban | −207.9 [−276.6, −144.0] | −206.7 [−259.2, −144.8] | 0/5 · 0/5 |
| Zero hold | Oban | −229.4 [−282.9, −136.5] | −221.8 [−264.4, −124.4] | 0/5 · 0/5 |

Median enqueue-commit goodput was 728 jobs/s off, 745 zero, 602 River, 957 Apalis, and 324 Oban. These lane medians can come from different groups. The paired results preserve the reversals and the effect of River's long final settlement in group 5.

The next table partitions each paired submit gap into disjoint wall intervals. The write columns use when the 128th row reaches each milestone. The jobs that set those milestones can differ. These columns measure workload progress. Per-job durations and intermediate rank crossings are in the replay output. Take the sum within a group before taking a median.

| Mesh arm | Peer | Producer | Commit to first entry | First to last entry | Last entry to last first write | Last first to last second write | Last second write to last settlement |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Off | River | +26.3 | +2.7 | +19.4 | +2.6 | +1.2 | −37.9 |
| Zero | River | +21.5 | +2.3 | +6.0 | +3.5 | +0.8 | −40.2 |
| Off | Apalis | +29.1 | −4.9 | +41.5 | +1.6 | +0.1 | −0.8 |
| Zero | Apalis | +24.9 | −4.9 | +40.8 | +3.0 | −0.04 | −1.2 |
| Off | Oban | −10.3 | −193.6 | −5.5 | +1.6 | −1.2 | −0.1 |
| Zero | Oban | −12.1 | −193.5 | −18.9 | +1.4 | −0.7 | +0.7 |

- **Producer.** Median submit-to-enqueue-commit time was 39.8 ms off, 35.7 zero, 14.1 River, 10.4 Apalis and 50.0 Oban. River captured read, decode, and enqueue-call edges. Read plus decode took 1.35–2.31 ms across its five arms, and call-to-commit took 11.93–15.59 ms. Mesh and Oban have the before-send and return edges of the warm broker plus the product-row commit. Apalis has the corresponding warm-session edges. Their archives do not split file handling, request encoding, client transport, and SQL within that producer interval. Return happens past commit and can overlap the first claim or handler.
- **Apalis.** Mesh's rank-1 entry lead is about 5 ms in the paired median. Mesh is behind by rank 112 and rank 128 in all ten pairs. Rank 64 to 128 took 76.1–95.3 ms off and 74.2–108.8 ms zero, versus 47.3–59.3 ms Apalis. Its last first and second writes are ahead in all ten pairs. Median per-job entry to first commit is 3.09 ms off, 3.56 zero and 1.16 Apalis. First to second commit is 4.32, 4.75 and 3.58 ms. These medians describe the per-job path beside the wave-level gap. Apalis' original claim clock is unavailable. The SQL exports the final `lock_at`, refreshed for the Running transition. In these arms it follows entry for 118–128 rows per run, so substituting it would reverse the apparent claim-to-entry edge.
- **River.** First claim times are within 1.1 ms of Mesh in every pair. River enters the first jobs quickly, but its rank-16 first commit is 42–69 ms later than Mesh from enqueue commit. Mesh is ahead in entries at rank 64 in all ten pairs. From rank 64 to 128 River takes just 39.5–53.2 ms, ahead of both Mesh arms in every matched group. River reaches the last second commit first in all five off pairs and four zero pairs. Its last-effect-to-last-terminal drain is 11.3–89.7 ms, with one terminal XID for 128 rows in groups 1–4. Group 5 settles 112 rows at 167.0 ms and the remaining 16 at 255.2 ms from enqueue commit, 89.7 ms after the last second write. Mesh settles the first 112 jobs sooner in all ten pairs. This stock River arm has a consistent producer and late-wave advantage and a variable last-terminal result.
- **Oban.** Commit to first claim is 179.8–200.8 ms. Off and zero lead at first entry by about 194 ms in the paired median. That first admission wait explains most of the end-to-end gap. Oban records 128 terminal XIDs in every arm. Its final-drain difference has mixed signs. SQL records the first claim and entry gap, but does not isolate which part of Oban's wake and dispatch cycle consumed that wait.

Mesh records a millisecond-resolution marker inside the SQL grant, before the claim commits and the actor spawns. River and Oban retain `attempted_at`. Mesh's first claim lands within 2.5 ms of enqueue commit in all ten arms. Median per-job claim-to-entry time is 4.51 ms off, 4.37 zero, 2.46 River and 4.30 Oban. Entry comes from the recorded handler clock. The first and second write clocks use PostgreSQL commit timestamps for the attempt and effect rows' XIDs. Terminal time uses the finalized queue tuple's XID. Within-run rankings compare equivalent milestones, and pairing subtracts each lane's relative duration. [Mesh and Oban ledger SQL](../../native/ledger.py) · [River SQL](../../go/evidence.go) · [Apalis SQL](../../rust/jobs.sql)

## What Apalis acknowledges

Across groups 1–5, Apalis used **14, 18, 16, 16, 17 terminal XIDs** for 128 native `Done` rows. In each group 124–126 rows share a terminal XID with at least one other row, with cohorts as large as 16. The raw SQL export matches the ledger's state, XID and commit timestamp. The application export independently matches all 256 application XIDs. Even inside one terminal cohort, the effect rows have separate XIDs and their second commits span up to 8.54 ms. There are 112–122 terminal rows committed before the last second write. Apalis durably acknowledges in groups throughout the run. A shared `xmin` proves grouping at the transaction boundary. The sealed counters do not identify the number of UPDATE statements per terminal transaction or supply a stock per-row-ack control. [Raw-export checks and counters](evidence.json) · [Stock worker and application commits](../../rust/src/main.rs)

Apalis' last terminal commit precedes even Mesh's last second application commit in **every** pair, by 24.1–93.4 ms off and 14.8–116.2 ms zero when aligned at enqueue commit. Its median per-job second-write-to-terminal lag is 4.11 ms versus 2.32 and 3.13 ms for Mesh. The measured lead is already present in application progress. Grouping may still help Apalis recycle capacity and advance later application commits. The data cannot price the effect of replacing its grouped ack with per-row transactions.

## How much terminal handling can buy

The measured runner publishes a success only from the scheduler's exit callback after unwind and cleanup. The dispatcher removes an off-mode attempt after confirmed settlement. Zero hold removes it after the writer returns a confirmed result. The SQL grant also checks durable executing capacity. The sealed timing ledgers have no per-job actor-exit callback, receipt, or slot-release timestamp. Post-second-write time therefore includes effect response and handler return, exit confirmation, completion dispatch, terminal SQL and commit. A slot can stay occupied after the durable terminal timestamp until the dispatcher learns the outcome. [Exit callback patch](../2026-11-23-ordinary-completion-writer/combined/replay/release-exit.patch) · [Frozen runner, writer, and schema source](../2026-11-23-ordinary-completion-writer/combined/combined-source.tar.gz)

Each scenario retains both ordered application commits. The table gives the median and range of milliseconds saved against the observed end on the same Mesh arm. Each row prices a separate scenario against that baseline. Adding rows would count the same terminal work twice.

| Scenario | Off savings, ms | Zero savings, ms |
| --- | ---: | ---: |
| Freeze all application timestamps, set each terminal endpoint to its second commit | 3.0 [0.8, 5.3] | 1.9 [1.3, 3.0] |
| Advance later cohorts when earlier slots become available, preserving recorded claim cohorts and inferred dispatch slack | 23.9 [19.7, 32.7] | 17.2 [14.6, 20.5] |
| Also remove all inferred dispatch slack, retaining cohorts and their order | 31.4 [27.8, 46.0] | 35.8 [33.9, 50.2] |
| Relax cohorts and ordering and pack the recorded preterminal residence perfectly into 16 slots | 57.3 [54.1, 80.6] | 48.7 [46.6, 67.0] |

The first row is the direct end-of-run cap with the upstream timeline held fixed. It uses `max(second_commit)`, even if a different job was the last to settle. The second row also grants optimistic feedback. It keeps the recorded admit order and jobs in each claim-time bucket. To admit a bucket of size *b*, enough previously admitted jobs must have settled to leave *b* of the 16 slots free. The model uses the later of that capacity frontier and the previous bucket time, then adds the observed offset above that gate. Each bucket shifts its entry and both write timestamps together, preserving the recorded write separation and per-job work duration from claim to second commit. Baseline replay gives back the exact observed end, with one claim marker adjusted within its millisecond bin across the ten runs. The optimistic replay releases a slot at the second commit itself. A genuine terminal commit still has to follow the existing exit confirmation and be learned by the dispatcher. Instant release is therefore a deliberately unattainable ceiling within the fixed-work, fixed-slack model.

Under that optimistic feedback replay, both writer modes still trail Apalis on both clocks in every group. The residual median commit gaps are **16.2 ms off** and **18.4 ms zero**, and submit gaps **43.4 ms off** and **48.9 ms zero**. Against River it could reverse all four off commit-clock losses and two of three zero losses, while leaving every observed submit-clock loss. The enqueue-commit clock removes Mesh's measured producer deficit from the comparison. These residuals assume recorded preterminal work and the inferred admission cadence.

The third row also erases observed nonterminal dispatch slack. This leaks in a second change and is deliberately outside a terminal-only budget. With that change, only one off pair and two zero pairs reach Apalis' commit endpoint, and all ten still miss its submit endpoint. The fourth row is a still looser mathematical floor conditional on unchanged claim-to-second-commit residence, using the larger of the longest job and total residence divided by 16, plus the first claim time. It gives even coarse marker buckets perfect packing and removes all later admission cost, so it cannot be operationally credited to terminal handling. The distinction matters because durable settlement and subsequent admission feed back into one another. A controlled change could also change DB contention and preterminal durations. These ledgers cannot establish a causal upper bound for such changes. [Per-arm model inputs, savings endpoints and paired residuals](evidence.json)

## Where to look within slot refill

The SQL separates two sources of preterminal work. The Mesh arms issue **29–33 off / 50–74 zero** claim calls after idle versus **9** Apalis `get_jobs` calls per arm. Those Apalis counters exclude other stock lifecycle queries. Mesh's role snapshot, which also includes idle, counts 42–46 off and 62–86 zero admitted V3 statements. They sum to 53.0–74.8 ms and 67.7–89.0 ms of server statement execution, respectively. The archived SQL locks the queue for capacity, renders and sizes the context, and grants rows before the claim lane hands them to the dispatcher for actor launch. The counts and the late-entry spread implicate repeated claim and launch as a material path to examine. Summed server time is not wall time and cannot establish how much delay comes from a smaller batch, query service, dispatcher wake, or actor launch. [Queue and application SQL counters](evidence.json) · [Frozen source](../2026-11-23-ordinary-completion-writer/combined/combined-source.tar.gz)

The Mesh fixture checks out from eight warmed application sessions separately for each INSERT. Each checkout of a warm session runs a health query. The sealed SQL captures exactly 256 health queries and 128 INSERTs of each kind, versus 256 application INSERTs in Apalis' counter. Apalis' fixture holds one checkout across its two autocommits and timer, with a pool of slots plus two shared with its backend. The Mesh health queries sum to only 0.65–0.86 ms of server execution per run. If checkout matters, it would be through transport, scheduler wait, and session turnover as well as SQL execution. Nothing in the capture isolates those costs. Keep the exit callback and both application commits when examining confirmed-grant-to-entry and entry-to-first-commit under backlog. Changing the terminal batch alone has no repeatable end-to-end gain in the sealed same-ELF study. [Mesh fixture and pool source](../2026-11-23-ordinary-completion-writer/combined/combined-source.tar.gz) · [Existing writer study](../2026-11-23-ordinary-completion-writer-native-repair/README.md)

## Order and replay

The first two groups ran in reverse orders, as did the third and fourth. The fifth used a distinct order. Apalis beat both Mesh arms while scheduled both before and after them, and the two large off gaps were in groups 1 and 4. The measured windows did not overlap and followed their recorded order. All 30 used the same Linux x86-64 boot, CPUs 0–1 and PostgreSQL mount. Every submit window had zero measured CPU steal and no sampled build process. Brackets contain 2–4 host samples and span 216–667 ms. The slowest zero arm, group 4, also had the highest bracketed I/O wait at 6.82%. The similarly slow off arms had 1.57% and 1.22%. The sampling supports the steal and order checks but cannot diagnose short-lived scheduling or I/O stalls.

Run from the checkout root. Both commands replay existing evidence offline. Ledger timestamps have integer microsecond resolution. The output labels other units in the field names. The evidence lists relative input paths, SHA-256 digests, and archive member hashes. The archived source paths cited above are `source/compiler/mesh-rt/src/jobs/{runner,store,trusted,completion_writer}.rs`, `source/compiler/mesh-rt/src/jobs/schema.sql`, `source/compiler/mesh-rt/src/actor/scheduler.rs`, `source/compiler/mesh-rt/src/db/pool.rs`, and `source/fixture/{main,writes}.mpl`.

```sh
set -o pipefail
r=tools/durable-jobs-bench/results/2026-11-23-exit-confirmed-short-gap
s=tools/durable-jobs-bench/results/2026-11-23-ordinary-completion-writer-native-repair
python3 -B "$s/replay/verify-timing.py" | cmp - "$s/evidence/verified-timing.json"
python3 -B "$r/replay.py" | cmp - "$r/evidence.json"
```
