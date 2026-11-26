# Replay the actor stack turnover experiment

Run from the checkout root on Apple Silicon. Use Rust 1.99.0, LLVM 21, Python 3.12 or later, and PostgreSQL 17.11 for the V1 diagnostic. Choose a fresh directory under `target/ordinary-actor-stack-turnover` if the original experiment directory already exists. Adjust the paths in the commands to match.

```bash
root="$PWD"
result="$root/tools/durable-jobs-bench/results/2026-11-23-ordinary-actor-stack-turnover"
(cd "$result" && shasum -a 256 -c SHA256SUMS)
work="$root/target/ordinary-actor-stack-turnover/replay-fresh"
mkdir "$work" "$work/ordinary" "$work/bin" "$work/tmp" "$work/observations"
git archive 3b20337ff7912c96362ef4a86ff32a4623226d6d | tar -xf - -C "$work/ordinary"

export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export RUSTUP_TOOLCHAIN=1.99.0 CARGO_NET_OFFLINE=true
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21
export TMPDIR="$work/tmp/" CARGO_TARGET_DIR="$work/cargo"
unset MESH_MOVABLE_STEP MESH_GROUPED_STEP

cd "$work/ordinary"
cargo build --locked --offline --release -j 5 -p meshc -p mesh-rt
mkdir "$work/bin/stock-rt" "$work/bin/reuse-rt"
cp "$work/cargo/release/meshc" "$work/bin/meshc"
cp "$work/cargo/release/libmesh_rt.a" "$work/bin/stock-rt/libmesh_rt.a"
bash "$result/replay/link.sh" "$work/bin/meshc" "$work/bin/stock-rt/libmesh_rt.a" "$work/bin/stock"

git apply --check "$result/stack-reuse.patch"
git apply "$result/stack-reuse.patch"
cargo build --locked --offline --release -j 5 -p meshc -p mesh-rt
cp "$work/cargo/release/libmesh_rt.a" "$work/bin/reuse-rt/libmesh_rt.a"
bash "$result/replay/link.sh" "$work/bin/meshc" "$work/bin/reuse-rt/libmesh_rt.a" "$work/bin/reuse"
cmp "$work/bin/stock.ll" "$work/bin/reuse.ll"
cc -Wall -Wextra -Werror -O2 -dynamiclib "$result/replay/native-calls.c" -o "$work/bin/native-calls.dylib"

cd "$work"
python3 -B "$result/replay/measure.py" baseline observations/original bin/stock --rounds 2
python3 -B "$result/replay/measure.py" timing observations/timing bin/stock --reuse bin/reuse
python3 -B "$result/replay/measure.py" memory observations/memory bin/stock --reuse bin/reuse --rounds 3
python3 -B "$result/replay/measure.py" counts observations/counts bin/stock --reuse bin/reuse --interposer "$work/bin/native-calls.dylib"
python3 -B "$result/replay/summarize.py" observations/{original,timing,memory,counts} > observations/report-data.json
python3 -B "$result/replay/price-v1.py" observations/report-data.json 	"$result/evidence/native-v1-dense-checkpoint.json" > observations/queue-budget.json
```

To recheck the original run, extract `evidence/actor-arms.tar.gz` beneath your fresh work directory and pass the four original directories to `summarize.py`. The archive includes each plan and every arm's JSON, stdout, and stderr. The first link attempt's runtime-filename error is preserved alongside the successful retry.

## Count a V1 dense fixture

Verify and unpack the retained `ordinary-trusted-v1.tar.gz` into `$work/v1/source`. Its SHA-256 is `e742644d89b872a41b4d9d241091a2349491f3de6d393fe77c4cbdbc05f6a58c`. The retained local artifact is in `target/ordinary-trusted-mesh-queue/local-artifacts-v2` when that earlier build is present. The retained executable SHA-256 is `16af053cf4aec1d6c8529ec615eb6b243a9caf135b526f4b3aa41c9e370ba06e`. The source archive contains the same fixture if you need a rebuild with a new artifact identity. [Archived V1 source](../2026-11-23-ordinary-trusted-v1/README.md)

Run only the local driver below. It starts a disposable loopback PostgreSQL and injects the counters into the eight Mesh runners. The archived native setup and dispatch scripts are unnecessary for this diagnostic.

```bash
mkdir -p "$work/v1/source" "$work/v1/state"
archive="$root/tools/durable-jobs-bench/results/2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz"
printf '%s  %s\n' e742644d89b872a41b4d9d241091a2349491f3de6d393fe77c4cbdbc05f6a58c "$archive" | shasum -a 256 -c
tar -xzf "$archive" -C "$work/v1/source"
cp -Rp "$root/target/ordinary-trusted-mesh-queue/local-artifacts-v2" "$work/v1/retained"
printf '%s  %s\n' 16af053cf4aec1d6c8529ec615eb6b243a9caf135b526f4b3aa41c9e370ba06e "$work/v1/retained/mesh" | shasum -a 256 -c

export PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin
python3 -B "$result/replay/v1-count.py" "$work/v1/"{source,retained,state} 	"$work/bin/native-calls.dylib" "$work/observations/v1-retained"

(cd "$work/v1/source" && git apply --check "$result/stack-reuse.patch" && git apply "$result/stack-reuse.patch")
mkdir "$work/probe"
cp "$result/replay/v1-count.py" "$work/probe/"
cp "$result/replay/v1-manifest.py" "$work/v1/manifest.py"
bash "$result/replay/v1-build.sh" "$work"
python3 -B "$result/replay/reduce-v1.py" "$work/observations/"{v1-retained,v1-reuse}
```

Supply Z3 4.15.7 at `$work/bin/z3` to replay the workspace and PostgreSQL gates with `bash "$result/replay/gates.sh" "$work"`. Use `scripts/verify-grouped-step-kill.py` from the snapshot with `--meshc`, the chosen `--runtime`, and LLVM 21 `--clang`. Run once with `--ordinary` and once without it.
