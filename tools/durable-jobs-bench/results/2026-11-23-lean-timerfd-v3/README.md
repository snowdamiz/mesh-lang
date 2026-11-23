# Protected lean timerfd native proof

The controller ran the exact signed timerfd request on November 23, 2026.
The original grant started at 22:10:18 UTC and released at 22:13:43 UTC with
exit 1. The runtime build and relink both succeeded. The watchdog process
ran as `nobody` and failed to import `component.py` from the execute-only
staging directory. The PostgreSQL proof and component rotations were
unreached. [Actual release](first/release.json) · [Raw evidence](first/remote.tar.gz)

The archive preserves all four linked AMD64 images and their exact source,
runtime, and handler object identities. The sealed object combination used
worker SHA-256 `51cf88a42254778ae659dd72c88d6ce0ce9b2c7917e929301e71f869f276af26`
and parent SHA-256 `8caef23b61f9bc8aa3bd9b7c9972b4c41226de749f57af568a898d63bb9c8d3c`.
The diagnostic ELF pair is independently pinned in the closure receipt.
[Exact artifact hashes](first/closure.json)

The build-free replay reuses those exact images and the original signed
scripts. It places the signed Python modules in a readable subdirectory and
checks the import as `nobody` before the watchdog cases. This first attempt
carries no child system, child total, or runner-tree comparison. See the
controller queue for the status of the separate replay.
