# Replay the slow runner experiment

Run from the repository root. This replays archived PostgreSQL ledgers and does not launch a database.

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-ordinary-slow-runner-concentration
lane=target/ordinary-slow-runner-concentration
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
mkdir -p "$lane"
replay=$(mktemp -d "$PWD/$lane/replay.XXXXXX")
tar -xzf "$packet/evidence/local-raw.tar.gz" -C "$replay"
python3 -B "$packet/local.py" replay "$replay/primary-01" > "$replay/summary.json"
cmp "$replay/summary.json" "$packet/evidence/local-summary.json"
python3 -B "$packet/diagnostic.py" "$replay/primary-01" "$replay/summary.json" > "$replay/sql.json"
cmp "$replay/sql.json" "$packet/evidence/local-sql.json"
python3 -B "$packet/offline.py" "$replay/primary-01/01/two" > "$replay/offline.json"
cmp "$replay/offline.json" "$packet/evidence/offline.json"
python3 -B "$packet/native.py" check > "$replay/native-offline-config.json"
cmp "$replay/native-offline-config.json" "$packet/evidence/native-offline-config.json"
```

For a fresh local run, extract the retained artifacts and use PostgreSQL 17.11 from Homebrew. The local driver acquires the existing queue-performance lock separately for each arm.

```sh
frozen=tools/durable-jobs-bench/results/2026-11-23-slow-storage-transfer/frozen
mkdir -p "$lane/base" "$lane/artifacts-v1" "$lane/scratch"
tar -xzf "$frozen/local-harness.tar.gz" -C "$lane/base"
tar -xzf "$frozen/local-compiled-v1-darwin-arm64.tar.gz" -C "$lane/artifacts-v1"
python3 -B "$packet/local.py" run "$lane/repeat-01" --wire
```

The native command requires a current TIMING grant and an identical intake in the sole controller's inbox. The offline check reads the pinned inventory and performs no network calls.
