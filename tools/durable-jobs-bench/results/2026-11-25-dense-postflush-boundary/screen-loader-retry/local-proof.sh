#!/usr/bin/env bash
# Linux loader correctness only. The stand-in byte file is never opened by PostgreSQL.
set -euo pipefail
cd "$(dirname "$0")"
image=3dc01f342e31ad8de39338d14ab12de0090c9102b34f75ca9774ff1f4f5e904e
python3 -B - <<'PY' | docker run --rm --network none --pull never -i --entrypoint /bin/bash "$image" -euc '
    ln -sfn /usr/local/bin/python3 /usr/bin/python3
    /usr/bin/python3 --version
    mkdir /fixture
    tar -xf - -C /fixture
    base=/var/lib/durable-jobs-bench/native
    source="$base/source-ordinary-trusted-v1-23fdb3f5530d"
    module="$base/postflush-pg17-65af9b85ab67/postflush.so"
    matrix=/var/tmp/mesh-durable-jobs-matrix
    stage="$matrix/postflush-112233445566-off-a"
    mkdir -p "$source" "$(dirname "$module")" "$stage/raw/frozen/adapter"
    tar -xzf /fixture/stock.tar.gz -C "$source"
    chmod 0711 "$matrix" "$stage" "$stage/raw" "$stage/raw/frozen"
    chown nobody:nogroup "$stage/raw/frozen/adapter"
    chmod 0700 "$stage/raw/frozen/adapter"
    printf "%s\n" "local stand-in for the stage path" >"$module"
    chmod 0444 "$module"
    sha=$(sha256sum "$module" | cut -d " " -f 1)
    install -m 0644 /fixture/"old-overlay.py" "$stage/arm_overlay.py"
    for file in boundary.py native-arm.py topology_arm.py identity.py verify_stage.py; do
        install -m 0644 "/fixture/$file" "$stage/$file"
    done
    cd "$source/tools/durable-jobs-bench/native"
    reduced=(/usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 /usr/bin/python3 -B)
    if "${reduced[@]}" "$stage/arm_overlay.py" > /fixture/old.log 2>&1; then exit 1; fi
    grep -F "ModuleNotFoundError: No module named" /fixture/old.log
    install -m 0644 /fixture/new-overlay.py "$stage/arm_overlay.py"
    probe=("$stage/verify_stage.py" imports "$source" "$module" "$sha")
    "${reduced[@]}" "${probe[@]}"
    chmod 0600 "$stage/identity.py"
    if "${reduced[@]}" "${probe[@]}" > /fixture/restricted.log 2>&1; then exit 1; fi
    grep -F "unexpected helper mode or owner: identity.py" /fixture/restricted.log
    chmod 0644 "$stage/identity.py"
    mv "$stage/topology_arm.py" /fixture/held-topology.py
    if "${reduced[@]}" "$stage/arm_overlay.py" "$source" "$base/builds/app-pool-39e79f4abb76/direct-observed" \
        "$stage/raw/frozen/adapter" "$module" off > /fixture/missing.log 2>&1; then exit 1; fi
    grep -F "No such file or directory:" /fixture/missing.log | grep -F topology_arm.py
    install -m 0644 /fixture/held-topology.py "$stage/topology_arm.py"
    if /usr/sbin/runuser -u nobody -- env -i PATH=/usr/sbin:/usr/bin:/bin LC_ALL=C.UTF-8 \
        /usr/bin/python3 "${probe[@]}" > /fixture/bytecode.log 2>&1; then exit 1; fi
    grep -F "expected Linux nobody and Python -B" /fixture/bytecode.log
    test -z "$(find "$stage" -name __pycache__ -print -quit)"
'
import sys
import tarfile

import retry

stock = retry.STOCK_ARCHIVE
files = {key: src for key, src in retry.FILES.items()
         if key in {"boundary.py", "native-arm.py", "topology_arm.py", "identity.py", "verify_stage.py"}}
files.update({"old-overlay.py": retry.ORIGINAL / "arm_overlay.py",
              "new-overlay.py": retry.HERE / "arm_overlay.py", "stock.tar.gz": stock})
with tarfile.open(fileobj=sys.stdout.buffer, mode="w|") as archive:
    for name, src in files.items():
        archive.add(src, arcname=name)
PY
