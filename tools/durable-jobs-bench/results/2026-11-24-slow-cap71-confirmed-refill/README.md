# Slow cap-71 completion and refill

**Status: held by the user.** The controller has no reservation for this pilot.
The public same-seal native series has separate authority.

This prospective pilot isolates exit-confirmed completion/refill on two
32-slot product runners. The historical cap-71 study used V1 Mesh
`e86640db`, which settled every job separately. Mesh reached the first
committed application write earlier than Apalis in eight of eight blocks.
At rank 192 it trailed in all eight. From the 64th to the 512th first write,
Mesh used 94–266 ms more wall time. Time outside handler entry through the
second committed write contributed a paired median 138.1 normalized slot-ms.
The ledger leaves SQL, dispatcher turnover, and unreserved slots combined.
[Eight-block replay](../2026-11-23-iteration-comparison/slow-equal-refill.md)

## What the transfer can test

The private short16 implementation calls the released group settlement and
checked set claim within one transaction. A confirmed exit can refill the
freed slot immediately after COMMIT is acknowledged. The same queue row still
serializes admission across both processes. The operation holds completion
row locks while claiming and rendering replacement argument JSON. A queue
lock timeout rolls everything back and sends the finished jobs through
ordinary settlement. An ambiguous COMMIT suppresses dispatch of the
replacement until lease recovery. The runner also skips fusion while a
separate claim is pending. These costs can consume the saved exchange.
The outside-handler interval prices several causes and supplies no expected
speedup. [Private short16 SQL and fault packet](../2026-11-24-short16-fused-refill/README.md)

The public and candidate fixture sources match for the handler, application
statements, and producer. Every Mesh lane also launches the **exact public
producer ELF** (`af2aca867a02…`) as its sole warm producer/observer. Each
runner image migrates its own schema, with the extra refill grant scoped to
the private worker. The untimed Darwin proof used the retained local
executables and PostgreSQL 17.11. On both schemas, the same producer
persisted 512 uncompressed 16 KiB payloads in one outer commit with eight
bulk calls (504 jobs) and eight scalar calls. Both schemas support the
checked set claim. In the private schema, two scoped clients each completed
and replaced eight jobs in one transaction and the persisted executing count
stayed at 64. A forced queue lock returned SQLSTATE `55P03` with unchanged
rows. Namespace pause, queue pause, lower cap, and restored cap were checked
against real persisted states. The initial probe's row-`xmin` assumption
failed and its correction distinguishes the physical subtransactions from
the single committed producer receipt. Local work used untimed
SQL and broker proofs only. The original inline broker timed out in two
concurrent local proofs. Both schemas then passed the exact broker protocol
in serial with the original 15-second per-reply watchdog. Use the native
gate to verify the canonical broker exchange under that same bound.
[Direct-file proofs](evidence/) · [Shared producer transport](TRANSPORT.md)

## Frozen pilot

- Two reversed orders, eight arms per order. Each arm has a full warmup and
  a fresh measured cluster. The controller reconciles every warmup before
  the first measured arm in its block.
- Two grouped and two fused arms share the exact private ELF from the
  native short16 SETUP. A separate grouped `b3557273` bridge uses the
  qualified public ELF. D2 is off throughout and 1,024 real app checkout
  probes are required. The public selected-product slow-equal work uses
  its own selection rule and series.
- Exact original cap-71 Oban, River, and Apalis drivers and executables.
  The peer wrappers and River binary are restaged by digest into the new
  run directories. Their launch arguments, lifecycle, and connection
  allocation are preserved.
- PostgreSQL 17.11, CPUs 0 and 1, default storage, cap 71 at server and
  database, TLS and durable settings. Mesh holds four warm app sockets per
  runner, proved by live socket ownership. Each arm requires 512 real
  100 ms holds, 1,024 independent committed application XIDs, and
  terminal XIDs disjoint from both application and enqueue transactions.
  Mesh retains its persisted 64-slot cap and each runner its 32 slots.
- The replay reports the 64th-to-512th committed first-write interval,
  first-write ranks, normalized outside-handler slot time, first submit
  through final durable commit, enqueue commit through final durable commit,
  terminal cohorts, actual fused replacements, statement costs, and
  physical connection and process evidence.

Advance to groups 3–5 only if **both** pilot blocks pass. On each of middle,
submit and drain, the slower fused duplicate must beat the fastest grouped
control including the public bridge by more than **5% and the largest
duplicate spread**. Both fused arms must reduce outside-handler time and
each must use fusion for at least 90 replacements. All peer invariants apply
to every arm. This is a continuation rule for a two-block pilot. Retain any
negative or failed attempts and the previous series.

## Controller operation after the hold is lifted

The packet rejects SETUP and TIMING while the user hold is active. The sole
Linode controller owns both grants. Keep this behind already-booked
work, the seven native short16 fault cuts, and the repaired short16
adapter qualification. The folder passed as
`candidate_setup` contains that released attempt's `status.json`,
`raw.tar.gz`, and `mesh.json`. The binder verifies their source, native
runtime, fault receipts, and teardown witness. SETUP installs the exact
overlay archive and runs the canonical producer and capacity proof using
the two retained Linux ELFs. There is no build. Holding the queue row across
a competing admission is exercised in this setup. No timing may start until
the build-free proof reconciles.

```sh
p=tools/durable-jobs-bench/results/2026-11-24-slow-cap71-confirmed-refill
python3 -B "$p/controller.py" check
python3 -B "$p/controller.py" bind "$candidate_setup"
python3 -B "$p/controller.py" setup "$candidate_setup" "$p/evidence/native/setup"
python3 -B "$p/controller.py" timing "$candidate_setup" "$p/evidence/native/timing" "$p/evidence/native/setup" 1 2
python3 -B "$p/controller.py" replay "$candidate_setup" "$p/evidence/native/timing"
```

The Mesh broker adapter checks for a final reply again if the broker exits
between EOF and process polling. This handles the clean-exit race observed
in public dense-stock warmup after all terminal commits.

The continuation uses a separate TIMING grant for groups `3 4 5`. The
request seals `source.tar.gz` and its executable overlay inputs, the
upstream full source archives, the retained peers, and replay drivers.
Checks, binding, and replay are offline. Native evidence stays under this
packet's own `evidence/native` directory.
