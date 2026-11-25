# Retry the native empty recovery gate

This retry fixes the launch and evidence paths for the released gate-stock failure. The staged contract no longer assumes a checkout path. The launcher runs a named Python script so a bootstrap failure identifies the script in its traceback. The observer replaces the gate progress JSON atomically after each successful case. [Original failure and release](../2026-11-25-empty-recovery-native/evidence/native/README.md)

The candidate SQL, probe, public ELF, compiler, PostgreSQL pin, host pin, workload, durability checks, and decision thresholds come from the original experiment. The retry has a distinct packet ID and staging paths. The sole controller runs group 1 under a matching assignment and consults the existing decision gate before booking either continuation. [Retry packet](controller-packet.json)

The local proof runs the staged shell, the original product launcher and its UID drop, the product entry point, and the original recovery gate against an owned PostgreSQL 17.11 cluster over TLS. It uses a local fixture backend for migration and supplies test answers for the three Linode host identity checks. There is no Mesh workload in the local test. It runs plain `EXPLAIN` and one real recovery per case, then checks statement calls and row counts. Both definitions passed all 24 cases, including concurrent recovery, committed history, and the layout guard. The failing-anchor check verifies the named script in stderr. The plans establish that the candidate has a one-time filter over the empty target scan. Native executor work and timing still need the controller's run. [Untimed receipt](local-shell-proof.json)

## Replay from a scoped export

Use the [original export procedure](../2026-11-25-empty-recovery-native/REPLAY.md) with this directory's `replay-inputs.sha256`. The manifest includes the original packet, source bundle, failed arm, and release. In the export, unbundle `../2026-11-25-empty-recovery-native/source-b3557273.bundle` under the source commit's local Git object-store prerequisite. Run `python3 -B controller.py check` from the retry directory.

To reproduce the local shell proof from the repository root, use the listed Docker image. The container installs its test tools, binds the checkout read-only, and removes its own PostgreSQL cluster and container on exit.

```bash
retry=tools/durable-jobs-bench/results/2026-11-25-empty-recovery-native-retry
docker run --rm --cpuset-cpus 0,1 \
  --mount "type=bind,source=$PWD,target=/checkout,readonly" \
  --entrypoint /bin/bash \
  3540c0362c809df084ffac0ae69fe794515f9de92692f02178bcdce0ef0302ac \
  "/checkout/$retry/local-shell-proof.sh"
```
