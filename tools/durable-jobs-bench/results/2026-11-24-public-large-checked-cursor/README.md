# Public large checked-cursor claim audit

The released `b3557273` runner pays a speculative set transaction before every
checked cursor turn in the 64 × 512 KiB fixture. The matched slow, sparse and
three dense topologies use set grants and have zero observed cursor calls.
Keep the consumer change on hold until the build-free diagnostic prices the
reduction on the final-grant and durable-settlement clocks.

| Storage | Set calls and rows, by block | Checked cursor calls, by block | Cursor rows, by block | Mesh drain range | Median drain gap to Apalis |
| --- | --- | --- | --- | --- | --- |
| Default | 227, 219, 243, 234, 235 | 63, 52, 75, 66, 69 | 118, 108, 128, 121, 124 | 657–712 ms | 517.125 ms |
| Symmetric EXTERNAL | 209, 205, 210, 216, 210 | 61, 58, 64, 69, 61 | 116, 114, 120, 124, 116 | 666–724 ms | 506.048 ms |

Every cursor call listed above followed one confirmed set fallback. All 64
grants in each block came from the cursor. The fixed parts of the native
advancement thresholds are 129.28125 ms and 126.512 ms, respectively.
Use the larger threshold if the new duplicate stock controls require it.

The [sealed audit](evidence/audit.json) pins the selected native blocks, their
original raw SHA-256s, matched peers, host and runner topology, original
fixture, product ELF and manifest. The default and symmetric EXTERNAL
profiles have separate inventories and receipts. Each Mesh large arm runs
eight runners and 64 aggregate slots with 32 app TLS sessions, a 96-session
PostgreSQL 17.11 cap, 4 MiB work_mem and durable writes. All 64 enqueue calls
are scalar in one durable producer transaction. The ledger verifies 128
independent application write XIDs and 64 separate terminal XIDs per block.
The EXTERNAL peer archives also contain before/after DDL and physical
readback for all 64 payloads. The original Mesh archive identifies the
column storage policy and payload SHA-256s. The diagnostic obtains Mesh's
physical size and compression per row after durable settlement and checks
those hashes against the archived payloads.

## Counting the fallbacks

In the source's checked set mode, only an acknowledged fallback runs
`next_due_ms` and then starts the checked cursor transaction.
`next_due_ms_since` consumes an acknowledged empty marker with a cutoff.
The role-scoped SQL tallies have neither counter eviction nor unexplained
BEGIN, COMMIT or ROLLBACK differences. The only archived failed claim
statements in these blocks are the separately counted set queue lock
timeouts. Those roll back and do not proceed to the cursor.
Subtract the two distinct probe counts from set result rows to obtain set
grant rows. For large, the remainder is zero. The remaining cursor rows
beyond the 64 grants reconcile with runner contention hints after removing
set timeouts. This accounting distinguishes one-row grants, empty
markers, fallbacks and cursor hints without classifying by row count alone.

The source first takes the queue row lock, checks capacity and namespace
pause, and preselects the ordered locked eligible prefix. A physical datum
size sum above 65,536 or compression on any of the three tested columns
disqualifies the set context before detoasting. A busy eligible row can
also produce the same marker. The archived marker itself carries no
reason. The existing cursor acquires fresh locks on the same capacity
fence and enforces the exact ordered reply budget, oversized markers and
row-generation exclusion. Its queue admission uses SKIP LOCKED. The stock
`claim_batch_with_timing` entry point uses a different client validation
and transaction path.

The SQL elapsed sums aggregate successful executes across sessions and
across producer startup, workload and teardown, including deliberate idle
polls and overlapping waits. The acquisition statement can hold a queue
lock through row receipt and COMMIT, but the sums do not place specific
fallbacks on the critical path. The matched Apalis drain gap gives a target
for the consumer work. River already trails Mesh on default drain. The
ledger separates final grant, settle and submit clocks, so the large
producer interval cannot contribute to a measured consumer gain.

## Scope after the screen

Any adaptive prototype belongs in runner-local state for each configured
queue. Use positive evidence of a large fallback before temporarily picking
the existing checked cursor on a subsequent request. Legacy, contention,
empty and oversize paths need distinct handling. The existing undifferentiated
fallback marker alone is insufficient for activation. Expire the preference
and return to set on a small prefix, empty grant or uncertain receipt.
A mixed-size qualification must exercise the transition in one live runner.
Do not carry a speculative transaction or lock into the cursor attempt.
Reuse the normal pre-claim due probe, start the lease age after that probe,
and keep wakeup and retry scheduling. The cursor still must validate rows
before the exact COMMIT tag and idle ReadyForQuery. A lost ack can leave
the durable claim executing and launches must wait for recovery.

If warranted after the screen, the pure causal benchmark arm would launch
the same public set configuration and call the already released checked
cursor directly at the top of the set-mode acquisition branch, after the
ordinary due probe and fresh lease start. Run it on the exact original
producer with same-ELF stock controls and separate storage series. No
product config or rollout is implied. A viable adaptive prototype must
also earn its own measured result after the bounded preference and mixed
size tests.

Prior work priced cursor prefix plans, large-context rendering, work_mem
and spill, and changed producer transfer boundaries. The public dense
audit examines the UPDATE query plan in small set grants. This screen
varies only speculative claim admission before the already checked cursor.
The held slow cap71 refill is independent.

- [Earlier cursor plan](../2026-11-23-claim-cursor-plan/README.md)
- [Earlier spill and physical storage study](../2026-11-23-large-storage-component/README.md)
- [Earlier rendering study](../2026-11-23-large-arg-render/README.md)
- [Public dense set plan intake](../2026-11-24-public-dense-set-claim/README.md)
- [Public large file-session work](../2026-11-24-public-large-file-session)

## Build-free diagnostic

Use the sole controller and its existing access and timing locks. No
relink, producer change, booked product/scorer edit or fresh binary. Each
storage packet requests its own two reverse-order blocks, using the
original profile's warmup and measured phases.

- Stock A and B invoke the byte-identical original launcher and producer.
- Thin adds one namespace, queue and worker-role branch to the set function
  in a disposable benchmark database. It issues the same exact marker
  before speculative queue and job locks. The runner still validates it,
  commits it, probes due, begins again and uses the checked cursor. The
  cursor definition and producer are byte-identical. The arm intentionally
  forces extra cursor probes in empty scans. It prices removing speculative
  SQL under altered lock competition and can understate or overstate a
  bounded preference. It retains the extra protocol exchanges, so it
  cannot price a pure checked-cursor bypass.
- Trace adds successful Execute duration logging for the queue role, with
  no parameter contents. It reconstructs the acknowledged set/COMMIT/
  due/BEGIN/cursor/COMMIT chain per backend and gives the union of observed
  set transaction intervals clipped to the pre-final-grant and pre-terminal
  windows. This is elapsed coverage, including wait, and the server logs
  cannot attest to client ReadyForQuery times. Trace perturbation is reported
  separately. It does not enter the advancement score.

Advance per storage policy only if **both** reverse blocks improve both
first commit to final grant and first commit to last settlement by at least
the maximum of these quantities, against the faster stock duplicate on
each clock.

- One quarter of the five-block median matched drain gap to the fastest
  native peer.
- The full range of the five archived Mesh drain controls.
- Twice the largest within-block stock duplicate spread on that clock.

The paired median submit-to-last-settlement gain against the faster duplicate
must also be nonnegative. A pass admits a pure checked-cursor attribution
arm and bounded preference prototype. Use at least five independently
matched native blocks to qualify the end-to-end patch. A failed screen
leaves the claim path on hold. The diagnostic does not book another lane.

Both intake files are parked in this directory for a later session. Their
offline checks passed. The archived native measurements are all stock runs.

## Replay

From the worktree root, audit the raw archives and controller intake
offline. Diagnostic `timing` requires the matching active sole-controller
grant and byte-identical inbox intake. These packet files are the handoff
for that controller.

```bash
d=tools/durable-jobs-bench/results/2026-11-24-public-large-checked-cursor
python3 -B "$d/audit.py" --complete
python3 -B "$d/check_trace.py"
python3 -B "$d/controller.py" check large
python3 -B "$d/controller.py" check large-external
python3 -B "$d/controller.py" replay large          # after a native grant completes
python3 -B "$d/controller.py" replay large-external # after a native grant completes
```

The local proof uses the sealed macOS executable and a disposable TLS
PostgreSQL cluster. It exercises compressed default and uncompressed
EXTERNAL payloads in the public runner, then observes the next small grant
on the same live runner through checked set SQL. It holds a cursor SELECT
until release, disconnects before COMMIT and proves retry spends attempt
one, and drops idle ReadyForQuery after an actual cursor COMMIT. It proves
no launch from the ambiguous claim, then a single recovered handler at
attempt two after the first lease expires. Both thin arms exercise the
same checked cursor boundary. The proof records receipts and XIDs only.

```bash
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin TMPDIR="$PWD/target" \
  python3 -B "$d/proof.py" target/public-large-cursor-new-proof
```
