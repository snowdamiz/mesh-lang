#!/usr/bin/env bash
set -euo pipefail
sha="${1:?sealed overlay SHA-256}"
[[ "$sha" =~ ^[a-f0-9]{64}$ ]]
base=/var/lib/durable-jobs-bench/native
overlay="$base/source-ordinary-trusted-large-${sha:0:16}"
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<"$base/native.lock"
flock -n 9
umask 022
stage="$(mktemp -d "$base/.ordinary-trusted-large.XXXXXXXX")"
trap 'rm -rf -- "$stage"' EXIT
cat > "$stage/overlay.tar.gz"
printf '%s  %s\n' "$sha" "$stage/overlay.tar.gz" | sha256sum -c
mkdir "$stage/source"
tar -xzf "$stage/overlay.tar.gz" -C "$stage/source"
adapter=tools/durable-jobs-bench/native
python3 -B - "$stage/source/$adapter/assembly-pins.json" <<'PY'
from hashlib import file_digest
import json
from pathlib import Path
import sys
for path, sha in json.loads(Path(sys.argv[1]).read_text())["dependencies"].items():
    with Path(path).open("rb") as stream:
        if file_digest(stream, "sha256").hexdigest() != sha:
            raise SystemExit(f"changed retained input: {path}")
    print(sha, path)
PY
if [[ -e "$overlay" ]]; then
    diff -qr "$stage/source" "$overlay"
else
    mv "$stage/source" "$overlay"
fi
printf '%s\n' "$overlay"
