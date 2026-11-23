#!/usr/bin/env bash
set -euo pipefail
lane="$PWD/target/jobs-admission-v3"
relative=target/jobs-admission-v3
base="$(cat "$lane/BASE")"
patch="$lane/admission-v3.patch"
ref=refs/heads/experiment/jobs-admission-v3
files=(
    compiler/mesh-rt/src/db/jobs_connection.rs compiler/mesh-rt/src/db/pg.rs
    compiler/mesh-rt/src/jobs/runner.rs compiler/mesh-rt/src/jobs/runner_tests.rs compiler/mesh-rt/src/jobs/schema.sql
    compiler/mesh-rt/src/jobs/store.rs scripts/fixtures/jobs-admission-v3-role-grants.sql
    tools/durable-jobs-bench/native/arm.py tools/durable-jobs-bench/native/run
    tools/durable-jobs-bench/native/build
)
: > "$patch"
for file in "${files[@]}"; do
    old="$relative/stock/$file"
    if [[ ! -f "$old" ]]; then old=/dev/null; fi
    (git diff --no-index -- "$old" "$relative/source/$file" ||
        test "$?" -eq 1) | sed -e "s|a/$relative/stock/|a/|g" -e "s|b/$relative/source/|b/|g" -e "s|a/$relative/source/|a/|g" >> "$patch"
done
export GIT_INDEX_FILE="$lane/tmp/candidate-index"
rm -f "$GIT_INDEX_FILE"
git read-tree "$base"
git apply --cached --check "$patch"
git apply --cached "$patch"
git diff --cached --check "$base"
tree="$(git write-tree)"
candidate="$(git commit-tree "$tree" -p "$base" -m 'Prototype demand-queued durable Jobs admission and optional success cohorts')"
previous="$(git rev-parse -q --verify "$ref" || printf '%040d' 0)"
git update-ref "$ref" "$candidate" "$previous"
printf '%s\n' "$candidate" > "$lane/CANDIDATE"
mkdir -p "$lane/native"
COPYFILE_DISABLE=1 git archive "$candidate" Cargo.toml Cargo.lock .cargo/config.toml compiler tools/skill/mesh \
    tools/durable-jobs-bench/native scripts/ci-x86_64-linux-gnu-clang scripts/jobs-parity.py \
    scripts/jobs-parity-profiles.json proof/jobs-parity/linux-regression \
    proof/jobs-parity/linux-warm-replay-20261122.tar.gz > "$lane/native/source.tar"
tar --no-xattrs -rf "$lane/native/source.tar" -C "$lane" CANDIDATE price.py edges.py
gzip -kf "$lane/native/source.tar"
rm "$lane/native/source.tar"
shasum -a 256 "$lane/native/source.tar.gz" > "$lane/native/source.sha256"
git show --stat --oneline "$candidate"
