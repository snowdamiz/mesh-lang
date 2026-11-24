#!/usr/bin/env bash
set -euo pipefail
bundle="$(cd "$(dirname "$0")/.." && pwd -P)"
lane="${1:?new absolute build directory inside the checkout}"
compiler="${2:?absolute path to the pinned release meshc}"
expected_compiler="$(python3 -B -c 'import json,sys; print(json.load(open(sys.argv[1]))["compiler_sha256"])'\
  "$bundle/evidence/mesh-build.json")"
printf '%s  %s\n' "$expected_compiler" "$compiler" | shasum -a 256 -c
mkdir "$lane"
mkdir "$lane/tmp" "$lane/inputs"
(cd "$bundle/sources" && shasum -a 256 -c input-pins.sha256)
tar xzf "$bundle/sources/bulk-enqueue-ce5e551d079e.tar.gz" -C "$lane" ordinary-v2
tar xzf "$bundle/sources/ordinary-cooperative-pg-portable-results.tar.gz" -C "$lane/inputs" \
  bench.py faults.py fault-fixture proof-bin audit-short-writes.c
tar xzf "$bundle/sources/gate-fixtures-3828f20d.tar.gz" -C "$lane/ordinary-v2"
(
  cd "$lane/ordinary-v2"
  patch -p1 -f < "$bundle/replay/cooperative-pg-on-v2.patch"
)
export PATH="/opt/homebrew/opt/rustup/bin:/opt/homebrew/opt/llvm@21/bin:$PATH"
export LLVM_SYS_211_PREFIX="/opt/homebrew/opt/llvm@21" TMPDIR="$lane/tmp/" CARGO_TARGET_DIR="$lane/cargo"
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_NET_OFFLINE=true
(cd "$lane/ordinary-v2" && cargo +1.99.0 build --release --locked -p mesh-rt)
runtime="$lane/cargo/release/libmesh_rt.a"
bash "$lane/ordinary-v2/replay/link.sh" "$compiler" "$runtime" "$lane/artifacts"
MESH_RT_LIB_PATH="$runtime" "$compiler" build "$lane/inputs/fault-fixture" --opt-level 2 --output "$lane/actor-fault"
mkdir "$lane/queue-fault-fixture"
cp -R "$lane/ordinary-v2/fixture" "$lane/ordinary-v2/replay" "$lane/queue-fault-fixture/"
patch "$lane/queue-fault-fixture/fixture/main.mpl" < "$bundle/replay/fault-variant.patch"
bash "$lane/queue-fault-fixture/replay/link.sh" "$compiler" "$runtime" "$lane/queue-fault-artifacts"
cp "$lane/inputs/bench.py" "$lane/inputs/faults.py" "$lane/"
ln -s ordinary-v2 "$lane/base"
(cd "$lane/ordinary-v2" && patch -p1 -f < "$bundle/sources/gate-ce5e551d-test-adapter.patch")
