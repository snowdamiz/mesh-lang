# Canonical public checked 8×8 logical-key diagnostic

The Linode TIMING grant released with exit 1 at 04:24:19 UTC on November 25, 2026. Both group 0 diagnostic arms exited 0 on the host. The sealed scorer stopped before writing a reconciled block at `score.py:64` when it compared runner contention hints with an unscoped PostgreSQL log count. The frozen replay retains the failed block and the hashes of both raw archives. Groups 1 and 2 stayed gated.

The stock and key logs each contain two queue lock timeouts from the intentionally blocked `dense_grant_probe` sessions. The stock log also contains one timeout from the measured `mesh` application session. Stock therefore has three total log timeouts and one measured runner hint. Key has two total log timeouts and zero measured hints. The matched workload contexts are one and zero, respectively. These counts diagnose the cause of the scorer stop. Native exit 0 on the two arms does not establish plan qualification or timing eligibility.

The original release is `controller/controller-release.json`. The raw archives are in `blocks`, and the scorer traceback and `final-replay.json` preserve the original disposition. The request, scorer and raw archives retain their original bytes.
