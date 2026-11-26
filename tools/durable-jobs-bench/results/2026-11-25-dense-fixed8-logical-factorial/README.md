# Dense fixed8 and logical-key factorial

The primary candidate is the released grouped-refill ELF in fixed8ms mode
with grant SQL SHA-256 `d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45`.
All five Mesh arms use the same ELF for broker, migration and runners, the
same durable PostgreSQL 17.11 settings and 32 hot application TLS sessions.
A and B both use immediate settlement and stock SQL. The other three arms
are logical key only, fixed8 only and the composition, respectively.
The earlier peer decisions remain closed.

## Native assignment

- Run both diagnostic smokes on the grouped-refill image. They reconcile
  the real dense workload and exercise the migrated and composed grants in
  fresh disposable clones using the actual TLS worker role.
- Cover rollback, token fencing, pause, skip-locked, fallback and a held
  capacity change. Contract from two occupied seats to one and preserve
  both committed leases. The grant reopens only once occupancy falls below
  the new cap. Keep the clone checks out of the timed arms.
- Then warm and measure A, key, composition, fixed8, B and the exact
  reverse. Each arm has two 32-slot runners on CPUs 0 and 1, 1,024 jobs,
  two independent application writes per job, the same hot-pool prime,
  exit-confirmed grouping and one independent durable enqueue.
- Reconcile actual PostgreSQL and runner occupancy, verified files, PID
  identity, two write XIDs per job, terminal XIDs and writer receipts from
  raw archives. Prior native held/lost receipt and teardown proof is scoped
  to the byte-identical runner, writer and stock settlement functions.
  Retain the new same-image grant and contraction receipts separately.

## Prespecified advancement

In each measured rotation, submit-to-terminal and enqueue-commit-to-terminal
must beat the faster duplicate by strictly more than both 5% and the
duplicate spread. On both clocks, the composition must beat the faster of
the two individual components by strictly more than both 10 ms and the
duplicate spread. Last effect relative to enqueue commit must be earlier
than both duplicates. Terminal XIDs must fall by more than 5% and the
duplicate spread. Runner lifetime CPU, matched PostgreSQL CPU and pinned
host CPU must each stay within the higher duplicate. Record PostgreSQL
bracket slack and the matched PID count. Any unmatched PostgreSQL PID
makes a pilot arm unscorable.

Only a clean controller release and two passing rotations qualify the
composition for a separate reservation of five fresh warmup and measured
Mesh/River/Oban/Apalis blocks using the exact 2×32 peer inventories.
Report paired block differences on the observed clocks, with the actual
archived workload and process receipts for every arm.

## Replay

From the checkout root, run the commands listed in the sealed packet.
The controller alone registers the exact bytes and performs intake.
The build-free staging path uses the original grouped-refill source archive
at `target/dense-grouped-refill-packet/source.tar.gz`. New grants, order
receipts, staged inventories, downloads, raw archives and reconciliations
are kept under this directory. Preflight covers both original native
exports and a replay of each peer warmup and measured fixture.
