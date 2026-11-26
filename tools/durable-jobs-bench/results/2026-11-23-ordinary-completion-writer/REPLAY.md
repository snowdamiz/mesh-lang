# Replay the ordinary completion writer

The source is the committed V2 archive plus `replay/completion-writer.patch`. The patch SHA-256 sets the candidate's artifact version. `local-build.json` identifies the recorded compiler, runtime archive, native binding, Mesh fixture, and executable. `local-fault-build.json` identifies the fault-only fixture linked to the same runtime. A relink matched the compiler, runtime, Mesh source, and native object bytes. The executable, native archive, and package manifest digests differed. The relinked executable reconciled all 128 frozen short jobs. Each build seals the digests of the files it actually runs. [Relink comparison](evidence/relink-check.json) · [Relink ledger](evidence/relink-confirmed/zero/ledger.json)

## Verify the saved data

Run from the checkout root.

```bash
study=tools/durable-jobs-bench/results/2026-11-23-ordinary-completion-writer
shasum -a 256 -c "$study/SHA256SUMS"
python3 -B "$study/replay/campaign.py" summarize "$study/evidence/rotations-r1-matched"
python3 -B "$study/replay/campaign.py" summarize "$study/evidence/diagnostic-r1"
python3 -B "$study/replay/campaign.py" summarize "$study/evidence/scalar-diagnostic"
```

The summaries can also be regenerated from the extracted [raw evidence archive](raw-evidence.tar.gz). The score recomputes the frozen input and each window from the PostgreSQL ledger.

## Build and run the private candidate

Run from the checkout root with PostgreSQL 17.11 and the compiler used for the frozen V2 handler. `replay/materialize.sh` expects a fresh directory.

```bash
root="$PWD"
study="$root/tools/durable-jobs-bench/results/2026-11-23-ordinary-completion-writer"
v2="$root/tools/durable-jobs-bench/results/2026-11-23-ordinary-trusted-v2"
lane="$root/target/ordinary-completion-writer"
mkdir -p "$lane/state/scratch" "$lane/tmp"
export TMPDIR="$lane/tmp/"
export COMPLETION_WRITER_SOURCE="$lane/source-replay"
export COMPLETION_WRITER_ARTIFACTS="$lane/artifacts-replay"
bash "$study/replay/materialize.sh" "$v2/ordinary-trusted-v2.tar.gz" "$COMPLETION_WRITER_SOURCE"
bash "$study/replay/build.sh" "$COMPLETION_WRITER_SOURCE" "$root/target/debug/meshc" "$COMPLETION_WRITER_ARTIFACTS"
python3 -B "$study/replay/campaign.py" run "$lane/matched-replay"
python3 -B "$study/replay/campaign.py" run "$lane/diagnostic-replay" --trace
python3 -B "$study/replay/campaign.py" run "$lane/scalar-replay" --scalar
```

The local driver uses the original V2 executable at `target/ordinary-trusted-mesh-queue-v2/local-artifacts`. To reconstruct that control, extract the unpatched V2 archive and use its original `replay/link.sh` and `replay/manifest.py` with the corresponding release runtime.

To build the fault fixture, copy the materialized source's `fixture` and `replay` directories into a fresh directory under the owned lane. Apply `replay/compiled-fault-fixture.patch` from inside its `fixture` directory with `patch -p0`. Link it with that copy's `replay/link.sh`, the same compiler and runtime, and the same `COMPLETION_WRITER_SOURCE_SHA256`. Then run the compiled fixture probes.

```bash
python3 -B "$study/replay/writer-faults.py" "$lane/wire-replay" "$lane/fault-artifacts/mesh"
TRUSTED_ADMISSION_POLICY=success_cohort TRUSTED_COMPLETION_WRITER_MS=1 python3 -B "$study/replay/v2-lifecycle-faults.py" "$lane/lifecycle-replay" "$lane/fault-artifacts/mesh"
```

## Controller inputs

The controller can use `controller-plan.json` to select the new candidate label, controls, and initial matched orders. `replay/native-arm.sh` takes the mode, extracted source, artifact directory, private state, and fresh output directory. Its artifact check requires Linux x86-64. Set `WRITER_COMMITTED_TRANSITIONS=1` for the separate transition and handler-return diagnostic. The arm uses the controller's existing CPU pinning and private PostgreSQL configuration.

The raw archive includes the pilot, the stopped early rotation, the matched timing runs, the separate diagnostics, and all fault attempts. The pilot first encountered an occupancy assertion that used pre-commit claim timestamps. The amended replay scores committed transitions in the diagnostic and uses application entry to terminal commit as an observed lower bound in timing arms. The first wire replay hit Python's stricter X.509 extension check for the disposable CA. The second preserved certificate chain and hostname verification and completed the fault cases.
