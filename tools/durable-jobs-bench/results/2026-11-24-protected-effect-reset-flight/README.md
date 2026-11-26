# Protected effect and reset in one PostgreSQL flight

**Closed at evidence review.** The same transport was already built and tested
with the frozen 1,024-job fresh-exec workload. In the complete warmed Linux
ARM64 guest comparison, the one-flight candidate fell from 1,167.13 to
874.89 jobs/s against its same-linker control, a 25.04% decline. Runner CPU
rose from 0.905 to 1.061 ms/job. The separate traced proof matched all 1,024
consuming waits and ordered write pairs in both arms. The comparison contains
one complete untraced rotation and screens out a throughput gain. The first
warmed rotation exceeded the archive transfer limit under high workstation
load. The usable rotation took place on a two-vCPU Apple Virtualization guest.
Its result gives no Linode timing estimate. [Original guest report](../../../../target/linux-queue-perf/RESULTS.md) · [Raw warmed receipt](../../../../target/linux-queue-perf/dense-warm-02.serial) · [Raw trace proof](../../../../target/linux-queue-perf/dense-pipeline-proof.serial)

The old patch appended a standalone simple-query `DISCARD ALL` immediately
after `Sync` in the same bounded, authorized TLS write and flush. It required
both idle responses before publication or reuse. Patch SHA-256
`df809e20721ff7c6b7300f3487feda782e12622735e1677e57d358d7496c2dc8`
applied to the `e905b1d0` transport, which matched `8526a5bd`. The replay
pins the resulting ELFs `7c1c1cf2` and `d0a1c2da` from the guest's own
input inventory. Earlier macOS screen timings for that exact change were
inconclusive for wall and combined CPU because the seeded rows accumulated
across arms. The current source has since changed Bind encoding and response
scratch storage. It still has the same two-flight protocol. [Original build provenance](../../../../target/linux-queue-perf/pipeline-applicability.txt) · [Current EffectSession](../../../../compiler/mesh-rt/src/db/pg/jobs_effect_session.rs)

## Native counts and elapsed bounds

The replay reads 25 measured raw archives from the five matched strict
admission rotations on the Linode. Every arm used CPUs 0 and 1, the same
dense input, and durable PostgreSQL 17.11 with TLS and a physical cap of 96.
Each protected Mesh arm had eight runners, 64 slots, 32 warmed TLS application
backends, and 1,024 distinct worker PIDs. Each job produced an attempt row
and an effect row in two ordered, independently committed transactions
before its own terminal XID. The archived native ledgers contain worker PIDs
and durable rows. The exact wait proof cited above is from the separate
guest trace. [Native archives](../2026-11-23-admission-v3-strict/blocks/) · [Replay output](evidence.json)

Every strict admission arm counted **2,080** exact `DISCARD ALL` calls for
`compiled_caller` in `pg_stat_statements`, with **32** startup resets.
The application-role counter after idle was **4,096** in each strict
admission arm and its protected stock control, with zero evictions.
That is **2,048** named operation calls and **2,048** full resets during
the drain. Each invocation puts the reset in a second client flight. Moving
its request removes **2,048** such boundaries per arm. Both application
commits and both reset commands per job would still execute and require their
own acknowledgments. [Reset counter SQL](../2026-11-23-admission-v3/admission-v3.patch) · [Application-role counter](../../../../scripts/jobs-parity.py#L333)

| Rotation | Strict admission drain (ms) | Protected stock drain (ms) | Oban drain (ms) | River drain (ms) | Apalis drain (ms) | Strict post-commit interval union (ms) | Strict final effect commit to final terminal (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | 4,779.833 | 5,180.341 | 1,540.220 | 1,155.797 | 827.719 | 4,727.874 | 7.872 |
| 2 | 4,746.421 | 5,541.288 | 1,428.071 | 1,082.346 | 870.128 | 4,707.710 | 15.508 |
| 3 | 4,927.915 | 5,195.541 | 1,689.031 | 1,060.612 | 867.839 | 4,896.875 | 4.981 |
| 4 | 4,842.328 | 5,354.461 | 1,448.569 | 1,112.493 | 868.252 | 4,743.846 | 14.132 |
| 5 | 4,664.042 | 5,176.676 | 1,416.835 | 1,143.929 | 856.346 | 4,598.721 | 17.557 |

For each job, the first reset follows the attempt commit and precedes the
effect commit. The second follows the effect commit and precedes terminal
commit. The union column gives the exact ceiling on distinct observed wall
time covered by any of these possible reset-wait windows. The windows also
cover dispatch, receipt delivery, the next write, exact wait, and settlement.
Their summed lane time is 40.258 to 46.445 seconds per arm. The archive
lacks timestamps for the effect's idle response, the outbound reset, and
the reset's idle response. The union cannot isolate the removable wait or
predict how the rest of the drain would reschedule. The final column covers
only the overall tail after the last application commit.

Even if every saved microsecond counted in full toward the drain, matching the
same-rotation peers would require an average of **1,581–1,657 µs per removed
boundary** for Oban, **1,719–1,888 µs** for River, and **1,859–1,982 µs**
for Apalis. These are break-even requirements with zero overlap, computed
from each paired drain difference divided by 2,048. A 5% drain reduction
would require an average of 114–120 µs per boundary under the same assumption.
The existing native archives cannot establish savings at either scale.

A related Linode trace on the earlier protected stock ELF counted
**3,495.756 CPU-ms** in 1,024 consuming child waits and at least
**2,908.029 CPU-ms** on Jobs PostgreSQL backends in the drain. Holding that
observed work fixed would require **3,201.892 ms** on two CPUs even if all
application and other costs vanished. Child CPU alone would take
**1,747.878 ms**. The trace increased full drain by 6.91% against its
adjacent controls. This fixed-work bound applies to that diagnostic arm.
The application-backend range of 1,816.660–1,823.239 CPU-ms covers both
fenced operations, both resets, and all other work on those backends.
[Observer qualification and raw Linode trace](../2026-11-23-native-worker-cost/README.md)

## Protocol and outcome contract

- The effect frontend sequence is Parse, Bind, Describe Portal, Execute,
  Sync. The success parser requires empty ParseComplete and BindComplete,
  the exact one-column text boolean description, one boolean DataRow,
  `SELECT 1\0` CommandComplete, and idle ReadyForQuery. For a recognized
  SQLSTATE at the receipt position (`55000`, `55P03`, `25000`, or
  `57014`), the parser requires idle ReadyForQuery before returning a
  rejection. The next exchange needs its own `DISCARD ALL\0`
  CommandComplete and idle ReadyForQuery. The native SQL counters count server
  execution. The strict acknowledgments above describe the source exercised
  by successful attempts. The raw ledgers contain no per-frame transcript.
  [EffectSession and Frames](../../../../compiler/mesh-rt/src/db/pg/jobs_effect_session.rs)
- `Frames` reads a five-byte header and exactly the declared body. The length
  field is bounded at 4,096 bytes including its four-byte length word. Its
  `MessageBuffer` keeps 256 inline bytes and allocates bounded body scratch.
  Unconsumed TLS plaintext stays in the shared rustls connection, including
  when the next exchange arrives in the same TLS record. Each parser starts
  its own limit of 32 NoticeResponse or ParameterStatus frames under the
  same deadline. The existing adjacent-response test creates a fresh
  `Frames` at the first idle boundary.
  [Frame reader](../../../../compiler/mesh-rt/src/db/pg.rs#L723)
- `Sync` closes the implicit transaction on success and resumes the protocol
  after an extended-query error. An invalid effect outcome, unexpected
  transaction status, reset error, partial response, or loss after
  submission leaves the checkout uncertain. `EffectAuthority` admits each
  nonblocking socket send under its mutex. Publication requires both
  acknowledgments and live authority under that same mutex. Revocation
  shuts down the attached socket, and the relay retires uncertain sessions
  without replay. The prior prototype can send the reset before it finds
  an invalid reply or loses authority. The full reset is separate SQL, and
  PostgreSQL rejects `DISCARD ALL` inside an open transaction. New wire
  fault and owner-death qualification would apply to any future candidate.
  [Relay outcome handling](../../../../compiler/mesh-rt/src/jobs/effects.rs#L682)

The controller intake is on hold with zero requested minutes. The current
work was archive and source replay. It produced no candidate or new fault
results. See the [controller intake](request.json).

## Replay

From the checkout root, use Python 3.11 or newer. The command reads the
original in-checkout Linode archives and the retained guest serial captures
without starting a workload.

```sh
packet=tools/durable-jobs-bench/results/2026-11-24-protected-effect-reset-flight
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
python3 -B "$packet/replay.py" | cmp - "$packet/evidence.json"
```
