# Public large FILE: prepare payloads before queue admission

Candidate `f3b3612d9` applies the `locked_prepare_pg17_v1` protocol from
`955b356b5` to the public trusted runner `b3557273d`. The runner is the same
one used by the af2aca86 8×8 individual-settlement FILE series. The protocol
stays opt-in with `MESH_JOBS_BATCH_PROTOCOL=locked_prepare_pg17_v1`.
[Patch](candidate.patch) · [Sealed request](request.json) · [Controller](controller.py)

## Why

In the five-rotation FILE series, Mesh's last claim stamp came 606–703 ms after
enqueue commit. Its last settlement was only 20–45 ms after that stamp. Apalis
settled in about 168 ms and Oban in about 506 ms. In 4 of 5 rotations, 48–56 of
the 64 Mesh jobs had no claim stamp when Apalis settled. The delay is in the
grants, not in the handlers. [Claim gap](evidence/file-session-claim-gap.md) ·
[Baseline receipts](evidence/file-session-baseline.json)

The stock set-based claim reads each 512 KiB context while it holds the
serialized queue lock. The candidate reads and transfers the contexts first.
It holds only exact-tuple row locks during that step. A short activation then
takes the queue lock to admit the prefix.

## What changed

- The PG17 SQL schema, installer, remover, and the `pg17_lock_exact.c` helper
  are byte-identical to `955b356b5`.
- `prepared_claim.rs` now uses the public runner's checked claim reader. That
  reader enforces typed columns, the `SELECT n` count, UTF-8, the 32 MiB reply
  limit, and an open transaction. It also gates dispatch on COMMIT.
- At startup the runner refuses to start without PG17, all three functions,
  and EXECUTE on them for the worker role.
- Runners always report `last_batch_confirmed_us`, the wall clock after the
  claim COMMIT is acknowledged. That gives a timing for the last grant with
  event logging off.
- Job contexts, ordering, the 64-slot cap across 8 runners, namespace and queue
  fences, attempt, token, and generation all use the stock code paths. The
  effect fence and the legacy claimants do too.

## Local correctness (untimed, macOS arm64)

- `cargo test --workspace --locked` on the committed candidate checkout:
  exit 0, 4,828 passed, 0 failed.
- `scripts/verify-postgres-jobs.sh --compatibility 17.11` on clean HEAD
  `f3b3612d9`: exit 0, 131 passed, 0 failed. This includes the
  claim-admission and set-based-enqueue wire proofs.
  [Gates](evidence/local-gates.json)
- Public TLS replay with the FILE broker: 64 jobs, 8 runners, 64 slots, and
  192 distinct terminal, attempt, and effect transactions. It passed in both
  prepared and stock mode with the same runner ELF.
  [Prepared](evidence/local-prepared/result.json) · [Stock](evidence/local-stock/result.json)
- Wire faults on the compiled public runner (`prove-wire.py`):
  [result](evidence/local-wire.json)
  - A truncated prepare reply rolls back with no activation.
  - A truncated activation rolls back with both jobs still available.
  - A lost COMMIT reply leaves both jobs committed as executing. The runner
    does not launch them.
- The first `prove-wire.py` run failed because of a matcher bug in the harness.
  The proxy counted the runner's `to_regprocedure` capability probe as an
  activation. Calls are now matched only by `FROM mesh_jobs.<fn>(`. The
  failure log is kept at `target/public-large-exact-prepare/local-wire-failed-probe-match.log`.
- The strict `955b356b` proofs are archived unchanged in [evidence/strict](evidence/strict).
  These include the legacy fence, stale edit, installer, SQL contract, and old
  wire cuts.

## Native plan (sole controller, behind existing bookings)

1. `setup`: one build-and-smoke SETUP grant on the Linode. It builds this
   bundle with the retained compiler and installs the SQL and the `.so` only in
   the task database. It then runs one 64×512 KiB prepared smoke with the full
   ledger.
2. `pilot`: two reversed groups,
   `control-a, prepared, exact-stock, control-b` and its reverse. The controls
   are the candidate ELF in stock mode. `exact-stock` is the original
   af2aca86 ELF. All use the same recovered FILE broker (`a5a2d566…`) and the
   same storage topology. The pilot qualifies only if, in both groups:
   - The last confirmed grant, drain, and submit each beat the better control
     by more than the larger of the control spread, 20 ms, and 5% (3% for
     submit).
   - The controls agree with `exact-stock`.
   - Pinned CPU per job is within 120% of the controls, peak runner RSS within
     125%, and temp bytes within 16 MiB of them.
3. `peers`: only after a qualifying pilot. Five rotations with control,
   prepared, exact-stock, River, Oban, and Apalis. The peer inventories are
   identical to the original series.

The historical strict `955b356b` request (`large-v3-955b356b`) is still
`ready` in the queue, with no grant or release. This request does not replace
it, and its order is unchanged. The separate 989.5 ms `1bdbebfd` smoke is not
reused as evidence here.

`python3 -B controller.py check` re-verifies the request offline. It checks
the pins, the bundle, the recovered broker, the duplicate controls, and the
three peer lanes.
