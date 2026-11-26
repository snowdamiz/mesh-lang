# SHORT16 fixed8 and refill-demand pilot

Keep the dense winner's Linux AMD64 ELF `d56d0e61…`, its persistent inline
broker and logical grant body `d388baa3…`. On the frozen short profile,
run 128 jobs with a one-millisecond handler hold, one runner, 16 slots,
eight warmed application TLS sessions and a PostgreSQL 17.11 cap of 20 on
CPUs 0 and 1. The broker executes one enqueue SQL call returning 128 rows
in one durable transaction. The same application hot-checkout policy and
checked bounded grant query apply to every Mesh arm.

This condition is absent from the controller ledger and historical native
series. The dense winner ran 1,024 jobs on two 32-slot runners. The public
fixed8 correctness SETUP and its planned bridge use a different public ELF
and keep the bridge on that same dense workload. Short fused refill used a
combined settle-and-grant operation. Short R2 used a writer-off or zero-hold
policy and a different image. Its released SHORT16 stock and explicitly
prewarmed River inventories are the peer inputs here, alongside short Oban
and Apalis.

The replay at parent commit `572943359` verifies ten historical raw arms.
Its modeled fixed 8 ms wait would displace 28–51 new grants from the original
grouped timelines. Flushing instantly at pressure would require 26–33
settlement groups. The runtime waits for real SQL, COMMIT receipts and actor
exit. Those counts justify testing real refill demand and supply no forecast
of the timing result.

## Controller order

1. Run all three diagnostics before either measured rotation. Reuse the
   released source, ELF, role and TLS pins, with no build. Check the compiled
   broker's 128-job receipt, one SQL call returning 128 rows, both independent
   writes, checked grants, drain and SQL/XID widths. The fixed diagnostic
   also runs the existing checked-grant semantics in a disposable clone.
   The refill diagnostic must record actual dispatcher-driven flushes.
   Replay the existing independent held/lost receipt and actor teardown
   witness against the exact source and runtime from the winning build.
2. Warm and measure immediate A, fixed8, refill-demand and immediate B, then
   repeat in reverse. The immediate controls use the same grouped-success
   runner, with its optional asynchronous writer disabled. Refilling still
   requires actual receipt confirmation and does not free an unsettled slot.
   Record the original submit and enqueue-commit clocks, first and last
   committed effects, SQL and runner flush counters, sampled and conservative
   committed slot occupancy, and runner, PostgreSQL and pinned-host CPU.
   Database commit timestamps give a lower bound on how long a slot remains
   held because the client can confirm settlement later.
3. Refill-demand is the prospective primary. In **both** reversed groups it
   must reduce submit-to-last-settlement and enqueue-commit-to-last-settlement
   by strictly more than 5% of the faster immediate duplicate and strictly
   more than the duplicate spread on each clock. Its last effect measured
   from enqueue commit must precede both duplicates. Each of runner lifetime
   CPU, matched PostgreSQL bracket CPU and pinned-host CPU must stay at or
   below the higher duplicate. Any unmatched PostgreSQL PID fails replay.
   Fixed8 supplies a mechanism comparison and cannot earn the peer slot.
4. Only after the pilot exits zero and replay passes both groups, register a
   distinct peer grant for five rotated warmup and measured groups. Use the
   exact short inventories already qualified for stock River, prewarmed
   River, Oban and Apalis. Report paired clock, effect and CPU differences.
   Leave peer enrollment closed if either reversed pilot misses.

Run `python3 -B tools/durable-jobs-bench/results/2026-11-25-short16-fixed8-refill/controller.py check`
from the checkout root. The packet pins the shared artifacts already retained
under the benchmark results and `target`. The sole controller registers the
exact packet and calls `intake SHA256` after earlier bookings have released.
`replay` checks fresh exports. Only a qualifying pilot permits `intake-peers
SHA256` under the packet's peer ID. Keep all new raw attempts and releases.
