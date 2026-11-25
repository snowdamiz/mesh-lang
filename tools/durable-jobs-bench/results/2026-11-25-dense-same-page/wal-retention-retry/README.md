# Dense fillfactor WAL retention retry

The original default-a warmup reached PostgreSQL shutdown after eight bulk inserts and 1,024 rows for each of the two application writes. The original observer then called pg_waldump against the live cluster's WAL directory after shutdown. The native log records one recycled segment at the shutdown checkpoint. The reader needed both segments spanning `0/1A37BB0` through `0/2052B28`. The export retained `FixtureError` at `observe.py:90 → jobs-parity.py:235 → jobs-parity.py:93` and omitted decoder stderr. The missing start segment is the root-cause inference from the archived bounds, checkpoint and stack. The separate untimed PG17.11 reproduction independently retired the required segment at shutdown and captured the old command's exact missing-file error. [Original evidence](../evidence/native/README.md) · [Untimed WAL receipt](evidence/wal-proof.json) · [PostgreSQL checkpoint source](https://github.com/postgres/postgres/blob/REL_17_11/src/backend/access/transam/xlog.c#L6856)

One warmup ran, zero arms qualified, and zero measured arms launched. The exported warmup has neither a final ledger nor a decoded WAL reduction. The original frozen scorer's `KeyError: 'hot'` came from passing the public grader a pilot lane map keyed by fillfactor labels. The sibling grader checks the native exit and reports the actual failure stack before calling the public grader. Successful arms supply a grading view with their real inventory under the public scorer's hot lane. The offline check exercises this view on the authentic previously qualified hot arm and replays the failed original block through the matrix selector. [Offline replay](evidence/offline-check.json) · [Qualification count](evidence/replay-original.json)

The observer now copies the exact fully flushed segment window into disposable scratch after the existing workload and before fast shutdown. It runs the pinned 17.11 decoder on that copy after verifying clean shutdown. The replacement retains decoder stdout and stderr separately. The WAL grader still requires 1,024 inserts, 2,048 updates and the original same-page, prefix, durability, timing and footprint gates. The terminal lease check keeps the original SQL and uses a receipt field that survives the frozen exporter. The original packet, booking, failed export, structural proof, workload, executable, CPU and schedule pins are unchanged. [WAL retention helper](wal_window.py) · [Controller packet](controller-packet.json)

From this directory, run the offline check and untimed two-insert, one-update reproduction.

```sh
python3 -B controller.py check
python3 -B wal_proof.py /opt/homebrew/opt/postgresql@17/bin
```


The sole controller can bind the sealed sibling to the original booking's recovery in the established order. Under its matching TIMING grant, run `python3 -B controller.py timing` from this directory. Export each attempt and save the original controller release for the sibling as `evidence/native/controller-release.json`. Then run `python3 -B controller.py replay`. The two complete passing blocks and the clean sibling release govern eligibility for peer pricing.
