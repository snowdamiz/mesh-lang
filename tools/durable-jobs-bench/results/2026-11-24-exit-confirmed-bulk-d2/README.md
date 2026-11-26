# Exit-confirmed bulk D2 transfer

This packet tests the 500 ms D2 checkout policy on the pinned r2 runner with the completion writer off. Both arms use the official bulk V2 broker and migrator, the same compiled handler source, and one runner with 16 slots, eight warmed application connections, and a PostgreSQL cap of 20. The short input contains 128 jobs in one bulk transaction. Each job commits two independent application writes before terminal settlement.

The candidate source changes `db/pool.rs` and `db/pg.rs` from the pinned r2 archive. `MESH_PG_HOT_CHECKOUT=1` lets a checkout skip the health probe for a connection idle under 500 ms. A connection idle for 500 ms or longer still gets the bounded probe before use. A failed statement retires a broken connection. The driver reports a server error received before a missing `ReadyForQuery` while treating the outcome as unconfirmed. The next operation can open a replacement. The measured arms keep the exit callback, claim and lease source, and fixture bytes from r2. [Source](combined-source.tar.gz) · [Two-file patch](d2-transfer.patch)

The prior bulk V2 plus D2 series and the r2 writer series retain their own controls and receipts. The frozen V2 stock-peer request has its own controller ID. [Original D2 series](../2026-11-23-ordinary-hot-checkout-bulk-v2/README.md) · [Original r2 series](../2026-11-23-ordinary-completion-writer-native-repair/README.md) · [Frozen V2 peer request](../2026-11-24-bulk-d2-short-peers/request.json)

## Local result

Throughput transfer is inconclusive on macOS. Across five reversed-order pairs, median paired throughput changed by +3.5% from first submit to last settlement, with a range of -6.9% to +41.2%. From first enqueue commit to last settlement, the change was +9.1%, with a range of -1.2% to +48.8%. D2 led in three submit windows and four drain windows. The shared host load varied, and the replay did not establish which resource limited throughput.

D2 reduced application checkout probes from 256 to 2 in every pair. Each arm completed 128 jobs without a failed or retried job, with 256 independent application XIDs and a peak of 13 connections. The candidate with the opt-in unset issued 256 probes. The stale-session test sent the retired backend one probe and zero application writes. The dropped-reply test saw the original application write commit and counted zero retransmissions before a separate job obtained a replacement session. Both claim and terminal fault cases passed against both runners. Forced process loss with the exit callback held left the job executing.

The workspace build and all three PostgreSQL Jobs test groups passed. The focused database suite passed 231 tests, and the scheduler test confirmed cleanup before forced actor exit notification. [Gate receipt](local-gates.json) · [Gate logs](local-gate-logs.tar.gz)

## Replay local evidence

From the checkout root, run the offline checks with Python 3.11 or newer.

```sh
r=tools/durable-jobs-bench/results/2026-11-24-exit-confirmed-bulk-d2
(cd "$r" && shasum -a 256 -c SHA256SUMS)
python3 -B "$r/replay/local-replay.py"
python3 -B "$r/replay/controller.py" check
```

The macOS replay uses the retained r2 Mach-O, the official local bulk broker, and `target/queue-performance.lock` for the timed pairs. It checks five alternating pairs and a candidate launch with the opt-in unset. Per-arm receipts record the normalized probe query, insert counts, TLS roles, application XIDs, terminal XIDs, and both throughput clocks. The teardown witnesses hold a native termination callback after both application commits. The process-loss witness kills the real candidate with the callback held and checks that the job is still executing with the same XIDs. The fault fixtures run on disposable PostgreSQL 17.11 and use fewer slots and application lanes to gate individual exchanges.

The source capsule contains the release build inputs. To repeat the Rust test gates, use the repository proof script and test fixtures from the frozen r2 verification checkout under `target/ordinary-completion-writer/combined/workspace`. Use a task-local Cargo target and temporary directory. [Local raw archive](local-raw.tar.gz) · [Local scores](local-summary.json)

## Run the native comparison

The central controller owns SETUP and TIMING for `2026-11-24-exit-confirmed-bulk-d2`. SETUP verifies the retained r2 Linux ELF and manifest, the official bulk broker and migrator, the host identity, and the cold stock peer files. It builds the candidate runtime, relinks one runner, and gates timing on fresh writer-off smokes and the four D2 fault cases. The raw SETUP archive records the live direct-child identities and the new ELF and manifest digests. The native teardown baseline comes from the original Linux proof. [Controller packet](controller-packet.json) · [Native setup recipe](replay/native-setup.sh)

After the successful SETUP release, use the TIMING grant to run groups 1 and 2 in opposite orders. After both groups reconcile, run groups 3 through 5 under TIMING. Each measured arm creates a fresh PostgreSQL cluster on CPUs 0 and 1. The peer lanes pin protected Mesh and stock River, Oban, and Apalis. The stock peer programs retain their existing inputs and entry points.

```sh
python3 -B "$r/replay/controller.py" setup
python3 -B "$r/replay/controller.py" verify-setup
python3 -B "$r/replay/controller.py" timing 1 2
python3 -B "$r/replay/controller.py" timing 3 4 5
python3 -B "$r/replay/controller.py" summarize
```
