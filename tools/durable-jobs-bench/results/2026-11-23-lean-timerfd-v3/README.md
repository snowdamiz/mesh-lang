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

## Build-free retry

The separately signed retry ran at 22:41:31 UTC and released at 22:41:53
UTC with exit 0. The original four ELFs were reverified. The preflight
imported the full chain as `nobody` from the exact Linux directory. Eleven
watchdog cases passed, including fragmented renewal, EOF, and a suspended
owner. The strict PostgreSQL proof reconciled four distinct worker PIDs
and exact consuming waits, ordered fenced effects and resets. PostgreSQL
recorded 28 admitted claim turns and zero cohort completions. [Actual retry release](retry/release.json) · [Watchdog cases](retry/proof/watchdog.jsonl) · [Strict PG proof](retry/proof/strict-pg/joined-evidence.json)

Five same-boot component rotations measured 256 attempts per arm on
CPUs 0 and 1. The paired saving is the median of each rotation's mean
duplicate lean control minus timerfd. Spread is the largest absolute
control-to-control difference across the five rotations.

| CPU per attempt | Paired saving (ms) | Duplicate spread (ms) |
| --- | ---: | ---: |
| Child system | 0.2054 | 0.1048 |
| Child total | 0.2178 | 0.0718 |
| Runner tree | 0.1236 | 0.0847 |

The parent used 0.0898 ms more CPU per attempt in the paired median. The
child-total and tree savings missed the predeclared 0.300 ms component
gate. The reduced data records the resulting full-workload eligibility
decision. [Reduced receipt](retry/proof/reduced.json) · [Complete retry export](retry/remote.tar.gz)

```sh
retry=tools/durable-jobs-bench/results/2026-11-23-lean-timerfd-v3/retry
python3 -B "$retry/replay/reduce.py" "$retry/proof/timing.jsonl" |
  cmp - "$retry/proof/reduced.json"
```
