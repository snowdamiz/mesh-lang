# Native application plan reuse recovery

The recovered Linode experiment did not meet the adoption threshold for an ordinary `Pool.execute` change. Over six rotations, the named arm saved a median 58.269 ms per 1,024 pairs against the faster stock control in each rotation. It beat both controls in five rotations and exceeded their spread in three. The frozen threshold was a 75 ms median gain and four rotations beyond duplicate spread. [Retry request](../../request-sampler-retry.json) · [Native summary](native-02-sampler-retry/summary.json)

| Arm | Median wall time | Range |
| --- | ---: | ---: |
| Stock A | 494.329 ms | 481.110 to 603.518 ms |
| Stock B | 485.834 ms | 478.927 to 1,722.659 ms |
| Named | 427.026 ms | 408.408 to 483.097 ms |

Planning dropped from 1,024 to 24 plans per INSERT statement in every named arm. Median combined client and backend CPU fell by 115.973 CPU-ms against the lower-CPU stock control in each rotation. The slower stock B arm in rotation three spent 1,602.335 aggregate ms in WAL sync. Each arm used synchronous commits on the same two pinned CPUs. Across 18 arms, the replay verified 36,864 committed rows in distinct transactions, zero write errors, and zero detected CPU steal. These figures cover the isolated libpq and PostgreSQL component. [Raw retry receipts](native-02-sampler-retry/raw.tar.gz)

## Controller attempts

The original controller grant started at 03:42:58 UTC on November 24, 2026. The driver exited 1 before the first sample because the runner PID loop rebound the `worker` sampler parameter to an integer. The export finished at 03:43:12.189709 UTC, and the controller released the grant at 03:43:13.050335 UTC. The original request and source retain their SHA-256 seals. [Original request](../../request.json) · [Original export](native-01/raw.tar.gz) · [Original exit status](native-01/status.json) · [Original controller release](native-01/controller-release.json)

The controller repaired the two references to the loop variable in its staged copy and replayed the export using the unmodified scorer against that verified copy. The retry grant started at 03:46:31 UTC. The export finished at 03:46:50.370950 UTC, and the controller released the grant at 03:46:51.747988 UTC with exit 0. The input manifests contain one changed digest, for `measure.py`. They retain the PostgreSQL and host pins, workload, six orders, and scoring criteria. The original staging tars remain under `target/app-plan-reuse` with the archive hashes recorded here. [Staging provenance](staging-provenance.json) · [Retry exit status](native-02-sampler-retry/status.json) · [Retry controller release](native-02-sampler-retry/controller-release.json)

The untimed proof exercises the original and corrected `run_arm` and libpq call sites with deterministic fixture responses. The original raises the same exception on the control and named paths, before either sample. The corrected copy reaches both sample boundaries in both stock arms and the named arm. Each arm uses four fixture sessions and 48 write calls. The named arm prepares two statements per session. [Fixture receipt](sampler-proof.json)

## Replay the archive

Run from the checkout root. These commands check the hashes, reproduce the untimed fixture receipt, and compare the recovered summary to a fresh offline replay of all 18 native arms.

```sh
audit=tools/durable-jobs-bench/results/2026-11-24-ordinary-app-plan-reuse/evidence/recovery-audit
(cd "$audit" && shasum -a 256 -c SHA256SUMS)
python3 -B "$audit/prove-sampler.py" | diff - "$audit/sampler-proof.json"
python3 -B "$audit/verify-recovery.py"
```
