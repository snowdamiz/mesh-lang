# Dense fixed8 writer fusion

Private follow-up to the released dense 2×32 fixed8 + d388 winner. The base
source archive is deb2a3bc49feae7df985fe895209c3db7d725cd4c9c1b9c8ba49c93a650dd6d8,
the released bridge ELF is d56d0e6112a94ff601bebf6f777061068ed2fe0f2f2da586963dd6e1326a220a,
and the d388 SQL is d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45.
The off controls run fixed8 + d388 on the candidate image. Fusion retains
the fixed8 age and width triggers, the split lifecycle and ordinary claim
sessions, hot TLS application checkout, and the released producer.

## Seat accounting

The dispatcher reserves at most eight seats at handoff, entirely against the
selected queue's charged parent cohort. It adds the ordinary lane reservation
to current occupancy before computing any debt due to local contraction. The
SQL and receipt decoder both subtract that debt from actual token-fenced
successes. The ordered d388 function independently checks committed global
capacity, pause, worker and payload eligibility. The writer validates the
completion bitmap and all claim rows before sending COMMIT. The dispatcher
receives replacements only after the idle COMMIT receipt, and transfers parent
seats under its existing state lock. Stale, held, cancelled and expired rows
do not contribute releases. Local stop and scale changes use the existing
confirmed launch checks and unstarted release path.

Failed or contended pre-COMMIT attempts go back to scalar settlement.
A d388 fallback marker leaves the ordinary exact-prefix path available.
After an uncertain COMMIT, validated provisional grants never launch. The
dispatcher keeps the assigned parents charged, locks the provisional row by
ID in a separate statement to fence the original transaction, then releases
the token as unstarted and reconciles the parent. The database retains its
lease recovery path for a runner that exits before cleanup.

## Native order and gates

Only the named controller can operate the host. No new capacity or parallel
booking is required.

1. Register the sealed packet byte for byte for each phase in its existing
   controller queue. Run the offline check first. The SETUP booking calls
   `setup-spec`, verifies the stdin archive digest, and invokes the emitted
   setup script while holding the local controller fence. The script takes
   both host fences, verifies host and toolchain, builds the native release,
   repeats the inherited eight baseline checks and runs the scoped fused
   fault suite. Export the entire attempt as the standard raw archive with
   paths relative to the emitted evidence directory. Keep the setup release
   receipt and retain any failed attempt.
2. Bind the release and raw proof. Run the separate positive diagnostic
   reservation with the same 1,024-job dense fixture and two 32-slot
   runners, on CPUs 0 and 1. Require both runners to adopt replacements
   under handler pressure, at least 128 total, and reconcile the exact
   d388 SQL, two app commits per job, occupancy, ACL and contraction.
   The timed phases require the diagnostic's clean controller release.
3. Run two pilot rotations. Warm and measure off A, fused, exact released
   bridge, off B, then reverse. Both must pass the gate before any peers.
   The same checks run at restore from raw archives. Any incomplete or
   failed run stays in the evidence directory.
4. After both passing rotations and a clean release, run five new matched
   blocks against the pinned stock River, Oban and Apalis on the same
   topology and durable contracts. Replay emits paired differences.

Each pilot requires at least 256 receipt-proved replacement seats, at
least 15% fewer standalone grant-bearing transactions and fewer total
grant-bearing transactions than either same-image off control. The test
also requires fewer standalone d388 SELECTs than the two off controls
and the original bridge. A grant-bearing turn counts a confirmed
nonempty receipt once. It does not count timestamp matches or empty
polls. The fused linkage identifies the parent's commit boundary in
the ledger, so a pre-COMMIT grant stamp does not charge the replacement
before the transaction commits.

On both durable clocks, the candidate must beat the fastest off or
bridge by strictly more than 5%, 10 ms and the same-image duplicate
spread. The same rule uses 3% for the middle 820 first writes, second
writes and terminal commits. Each off control must remain within
the larger of 5% of the original bridge or the duplicate spread.
Last effect must move earlier and terminal XID count must stay within
the comparison envelope. Runner CPU, matched-PID PostgreSQL CPU,
pinned-host CPU and queue SQL calls must each stay within the higher
off control and 103% of the actual bridge. Statement elapsed time is
reported separately from process CPU.

Use the packet's commands from the checkout root. `dispatch` requires
the active controller grant, plus each prior clean release. `replay`
runs without contacting the host. The controller owns remote staging
and cleanup after verifying release and exporting evidence. All local
artifacts stay under this task's target directory.
