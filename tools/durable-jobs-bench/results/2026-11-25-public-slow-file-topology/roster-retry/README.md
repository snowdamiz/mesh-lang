# Public SLOW TLS roster recovery

The original SETUP released exit 1 at 04:14:27 UTC. All four diagnostic arms loaded the exact public runner and FILE broker, then stopped before enqueue when the reduced UID tried to enumerate the runner’s file descriptors. At the failure, the stock shapes had 13 TLS sessions and the split shapes had 17. The runner hardens the Linux process by disabling dumpability. Its readiness identity witness had already succeeded through the pinned owned-launch path. [Original release and four archives](../evidence/native/setup/)

The recovery uses PostgreSQL TLS rosters across serial runner launches. The borrowed dense witness measured 21 then 41 sessions on the exact native diagnostic. A later dense arm failed when the legacy early audit saw six participant sessions and the full roster expected eight. This sibling waits for the exact full roster and writes the standard TLS receipt from that same snapshot. The separate cap-71 SQL retry demonstrated a stable aggregate 17-session roster, durable worker identity and 64 total seats. The new prefix snapshots independently attribute four app sessions to each split runner at cap 68. [Dense recovery](../../2026-11-25-dense-live-grant-plans/procfd-recovery/) · [Cap-71 SQL audit](../../2026-11-24-ordinary-slow-equal-topology-cap71/SQL-AUDIT-RETRY.md)

The directly launched broker is the sole producer and SQL observer. Its initial roster is one session. The stock runner must add eight application and four participant sessions. Each split runner must add four of each. Every connection has a unique backend PID and client port, exact role, start timestamp and verified loopback TLS. The original role caps and the server and database cap of 68 are checked on every snapshot. Just after the original idle period, the adapter checks the owned process identities and requires the same exact SQL roster before the original submit clock can start. An underfilled full roster, a replacement with the same session count, a wrong role or a changed cap prevents submit.

## Untimed proof

The staged code ran as UID 65534 from a nonlistable Linux directory with isolated PostgreSQL 17.11 TLS and real libpq children. Disabling dumpability reproduced the permission failure in the original socket-map function for each child. The replacement matched the direct libpq application ownership counts of eight and four plus four. It rejected nine cases per shape, including a real connection replaced by a new backend at the same total count. The control stands in for the Mesh runner clients and exercises staged launch sequencing, audit and pre-submit methods. Native SETUP binds the exact retained Mesh ELFs and the full durable job lifecycle. [Original DB and procfs failure replay](controller-packet.json) · [Local lifecycle receipt](evidence/lifecycle.jsonl)

## Native custody

Only the sole controller stages or grants this sibling. The four diagnostic arms, cap 68, binaries, normalized input, readiness and idle settings, original scorer and pilot and peer gates remain pinned. The added replay check requires the SQL roster, the standard TLS receipt, the durable submit boundary and the original per-runner ownership field to agree. A clean SETUP is required before either timed phase. The distinct cap-71 fused hold remains in force.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-slow-file-topology/roster-retry
bash "$d/lifecycle.sh" | cmp - "$d/evidence/lifecycle.jsonl"
python3 -B "$d/controller.py" check
(cd "$d" && shasum -a 256 -c SHA256SUMS)
```
