# Verify the archive and replay locally

From the repository root, verify the recovery archive.

```sh
result="$PWD/tools/durable-jobs-bench/results/2026-11-23-ordinary-scheduler-supply"
(cd "$result" && shasum -a 256 -c SHA256SUMS)
python3 -B "$result/verify-recovery.py"
```

This checks the recovered receipt against the transcript output and verifies the observed rotation and selected evidence. The archived `summarize.py` expects full arm ledgers and can reconcile a fresh run. Its input contract excludes the compact historical receipt.

## Prepare a fresh scratch directory

The recovered drivers expect to run from a child of the repository's `target` directory. Copy them to a new directory and check the preserved pins before any timing. Use macOS on Apple Silicon, Python 3.12 or later, and Homebrew PostgreSQL 17.11. The original drivers use the existing local timing lock and the retained `target/ordinary-trusted-mesh-queue` source and executable. If the executable is missing or its SHA-256 differs, stop the exact replay. The tracked V1 source archive preserves the source for a separately labeled rebuild. [V1 source archive](../2026-11-23-ordinary-trusted-v1/README.md)

```sh
lane=$(mktemp -d "$PWD/target/ordinary-blocking-scheduler-replay.XXXXXX")
mkdir -p "$lane/evidence" "$lane/local-state/tmp"
cp "$result"/replay/*.py "$result/replay/pins.json" "$lane/"
PYTHONPATH="$lane" python3 -B -c 'from run import check_pins; check_pins()'
```

## Run a new local series

```sh
python3 -B "$lane/run.py" "$lane/evidence/local-replay-01"
python3 -B "$lane/summarize.py" "$lane/evidence/local-replay-01" \
    > "$lane/evidence/local-replay-01-summary.json"
```

Use a fresh name for each run. The resulting measurements belong to the new run. For separate instrumentation after timing, run the commands below. The saved `profile-first.py` records the initial 1 ms launcher. `profile.py` uses the successful 10 ms setup.

```sh
python3 -B "$lane/profile.py" stock-a "$lane/evidence/profile-stock-replay-01"
python3 -B "$lane/profile.py" workers-8 "$lane/evidence/profile-eight-replay-01"
```
