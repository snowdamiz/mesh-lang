# Dense grouped success at refill

The candidate is a portable overlay on the public grouped-hot fixture from
`b3557273d72be3802bf189b284e7caaf5f3caa0f`. Its async policies are opt-in.
The stock arm uses immediate public grouping with the same executable,
producer, split claim lane, and hot application checkout. [Candidate patch](candidate.patch)

## Prior work

The combined R2 writer already used exit-confirmed grouping, a separate
writer at zero hold, and the lifecycle session handoff. Its 0–2 ms option
used width and elapsed time without feedback from the dispatcher’s claim
reservation. The dense two-runner R2 test kept the writer off. The later
matched hot R2 screen tested zero and 1 ms, and both reversed groups missed
advancement. [Combined R2](../2026-11-23-ordinary-completion-writer/combined/README.md) · [Dense zero hold](../2026-11-24-dense-two-runner-completion/README.md) · [Native hot screen](../2026-11-24-dense-hot-completion-hold/load-retry/evidence/README.md)

Short16 fused refill coupled completion and grant inside one transaction
and changed the claim SQL. The separate dense confirmed-grant launch
experiment targeted adoption of a claim receipt on the individual-terminal
runtime. Repository history, archived source and reports showed no prior
native 8 ms group-success policy with the dispatcher’s claim reservation as
a flush signal. The earlier 8 ms fixed and refill arms are ledger models.
The six grouped-hot arms measured 869–906 terminal XIDs. The modeled
244–257 at refill assumes instantaneous confirmation at historical grant
stamps. [Short16](../2026-11-24-short16-fused-refill/README.md) · [Confirmed claim launch](../2026-11-25-dense-claim-lane-1e70f18d/README.md) · [Ledger replay](../2026-11-25-dense-completion-at-refill/README.md)

## Dispatcher and settlement

The writer collects fresh successes from the existing exit-confirmed
acknowledging state in cohorts of at most eight. It caps the collection
timer at 8 ms from the oldest exit stamp and shortens it at stop, queue
pause, or the member lease/renewal boundary. A cohort already in flight can
delay a later handoff. The pilot retains maximum observed collection age.

The refill arm dispatches a permitted claim first, using the ordinary
reservation against confirmed free slots. The claim request and a due queue
at its local cap are the two handoff signals. A due saturated queue records
the dispatcher’s intent under existing notification and backoff rules.
PostgreSQL still determines whether claimable jobs exist. There is no
work-ahead SELECT or change to the committed database cap.

The writer borrows the existing lifecycle session. While that session awaits
a receipt, the dispatcher continues to use the split claim session, and
renewals and scalar outcomes can use the existing one-session queue pool.
An in-flight member retains its attempt and its local slot. An unconfirmed
writer response uses the existing scalar token-fenced acknowledgement retry.
An admitting runner waits for a confirmed settlement or stale response
even past the local lease deadline. During global stop, the existing local
deadline can end an unresolved retry after admission is closed. Bulk
rejections and skipped members use the existing scalar path.

## Verification and pilot

The local Darwin/ARM64 proof used a compiled public fixture and a separate
terminate-callback fixture linked to the same patched runtime. Eight
disposable PostgreSQL checks covered SQL token and expiry fencing, held idle
ReadyForQuery, renewal through the hold, receipt loss and stale resolution
for scalar and bulk, cancellation and retry, normal and forced termination,
and partial stop. The held receipt case filled three genuine free slots
while the fourth stayed charged. The bulk loss submitted one two-member SQL
call and received two token-stale confirmations. Local receipts redact lease
tokens. [Fault receipts](evidence/local/) · [Build identity](evidence/local/build.json) · [Rust test results](evidence/local/test-results.json)

Native timing uses two runners with 32 slots and two scheduler workers each,
32 warmed application TLS sessions, pinned CPUs 0 and 1, PostgreSQL 17.11
with durable defaults and a 96-session cap, and the archived 1,024-job input.
Every lane warms up before the reversed measured orders. The replay checks
both clocks, last effect, the middle 820 effects, runner and pinned-host CPU,
independent application XIDs, per-runner and committed total occupancy,
terminal XID widths, bulk SQL, and claim SQL. Async lanes also report actual
dispatcher reservations and overlapping writer activity. A demand flush
can coincide with the timer or width limit. PostgreSQL statement elapsed time
includes waits.

Both blocks must beat the faster duplicate grouped control by strictly more
than 5% and the duplicate spread on submit and drain. Last effect relative
to enqueue commit must be earlier than both. Terminal transactions must fall
by more than 5% and the duplicate spread. The zero and fixed 8 ms lanes
separate async overlap, time-based collection, and demand feedback. The
public grouped stock plus River, Oban, and Apalis comparison requires two
passing pilots and an exit-0 controller release.

## Controller handoff

From the checkout root, run the host-free preflight.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-grouped-refill
python3 -B "$d/pilot.py" check
```

The sole Linode controller takes SETUP and then a separate build-free TIMING
assignment after the existing queue. The [sealed request](request.json)
names the source archive, phase IDs, scope, evidence locations and commands.
`setup-spec` emits the pinned setup script and stdin archive only during the
matching assignment. The controller retains the local timing fence while
invoking that script on the pinned host. The script takes the two remote
fences and builds and proves the candidate. `bind` replays the released raw
SETUP and gates all timing. The candidate’s stock, zero, fixed and refill
modes all use the resulting ELF.

For a failed or ambiguous attempt, retain all prior raw exports, grant and
release records. Check the original host identity, live children and all
three fences before considering a fresh controller assignment. Keep partial
blocks and choose complete blocks through replay. A failed advancement gate
ends this pilot campaign at the grouped controls.
