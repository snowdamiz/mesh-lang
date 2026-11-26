# Public dense hot checkout with actor readiness

This experiment tests whether actor-owned PostgreSQL readiness advances submit-to-terminal, enqueue-commit-to-terminal, and middle application-write completion on the public trusted runner at `1e70f18d624a841740d3bbb3d0fd09b269257f68`. The [previous native permit screen](../2026-11-24-dense-postgres-permits/README.md) ran 2,048 checkout probes before the 2,048 application writes. The [hot synchronous dense runs](../2026-11-24-dense-product-d2-transfer/CONFIRMED-REFILL.md) still had confirmed work waiting to enter a handler for a substantial part of the middle window.

The runtime patch reuses the original actor readiness reactor and the socket wrapper under rustls. Both checkout paths use the public runtime’s 500 ms rule. Readiness parks the actor on a stale probe or an application statement while retaining its exclusive lease. A stale probe may retire and replace its connection before SQL submission. An application failure returns to the handler without replay. Forced actor exit unwinds the registration and drops the transport without sending Terminate. PostgreSQL may still commit a write after local retirement or job cancellation. The proof records the terminal state while the second INSERT is blocked, then checks the effect’s independent commit if PostgreSQL completes it.

## Shared logger-off fixture

The amended requests are in [logger-off](logger-off/). They replace the parked
original reservation. Both public dense experiments link the same
[canonical fixture](../2026-11-24-dense-individual-hot-pool/logger-off-retry/main.mpl),
SHA-256 `97b8e75f8568191ac2b3a3160ce9c40d0eebc822ba2ec4661259510734816661`.
Its only delta from the original Jobs proof is deletion of the hardcoded
`event_log:true` option. The original logged proof and
[original packet](evidence/original/packet.tar.gz) remain available.

The amended setup uses its own candidate runtime and reruns the TLS and
lifecycle proof on the exact ELF used for the bridge and timing. The bridge
permits socket-wait traces. Every warmup and measured Mesh transcript must
contain only readiness and its final drained receipt, with no logger buffer.
Replay rechecks the raw transcript before accepting an arm.

## Native sequence

All performance runs use the assigned Linode, CPUs 0 and 1, PostgreSQL 17.11, and the canonical 1,024-job dense input. Each Mesh arm uses the same release ELF for migration, the warm bulk broker, and two 32-slot runners. Each runner has two scheduler workers and 16 warmed TLS application sessions. The PostgreSQL cap is 96. Set-based checked admission and one terminal transaction per job are verified. Checkout switches apply only to the application URL. The readiness path has no extra software permit limit.

1. The sole controller takes a SETUP grant after existing bookings. It builds and pins the ELF from the [original public source and handler archive](../2026-11-24-qualified-hot-pool/source.tar.gz) plus [the runtime delta](runtime.patch). Untimed proofs cover verified TLS reads, healthy and dead stale checkouts, a lost application commit receipt, cancellation with a released server lock, and a killed runner. A single cold readiness diagnostic then verifies traced suspension in both runners and 2,048 checkout probes. The bridge’s trace makes its elapsed times diagnostic only.
2. The sole controller takes a TIMING grant for two pilot rotations. Each rotation uses fresh warmup and measured clusters for both identical hot synchronous controls and the hot readiness candidate. The second reverses the first. Scoring reconciles the actual 2,048 independent application write XIDs and 1,024 individual terminal XIDs in each arm. Runner CPU, pinned host CPU, and runner peak RSS come from the same native arm.
3. Each pilot must beat the faster synchronous control by more than both the duplicate spread and 5% in submit, drain, and 103rd-to-922nd second-write commits. Final effect must commit earlier relative to enqueue commit. Candidate runner and pinned host CPU per job may exceed the worse control by up to 5%, and peak RSS by up to 10%. If both pilots pass, a separate grant runs two prospective reversed rotations including the same Mesh arms and all three pinned equal 2×32 peers, Go River, Elixir Oban, and Rust Apalis. Replay reports each clock comparison and resource use.

## Controller commands

Run from the repository root. The amended setup and timing requests are byte-sealed in `logger-off/`. The controller owns the existing inbox, capacity grant, host locks, and release records. Record each successful release as `controller-release.json` in the request’s evidence destination. The controller retains raw archives and logs after failures and reconciles host and lock state before any retry.

```sh
d=tools/durable-jobs-bench/results/2026-11-24-dense-hot-readiness
(cd "$d" && shasum -a 256 -c SHA256SUMS)
python3 -B "$d/controller.py" check

# Sole native controller, under the exact matching active grants
python3 -B "$d/controller.py" setup
python3 -B "$d/controller.py" timing pilot
python3 -B "$d/controller.py" timing peers

# Offline replays of retained exports
python3 -B "$d/controller.py" replay setup
python3 -B "$d/controller.py" replay pilot
python3 -B "$d/controller.py" replay peers
```
