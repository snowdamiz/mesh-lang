# Local pool correctness

The receipt binds the macOS ARM64 checks to source revision
`1e70f18d624a841740d3bbb3d0fd09b269257f68` and compares the compiler,
workspace manifests, scripts, fixtures, and documentation against integration
commit `916cfe532d76616ea28856ff9ca8a5132fecc556`. Log digests cover the
decompressed, sanitized bytes. The original logs and the two checksummed
PostgreSQL source tarballs remain in `target/native-qualified-pool-transfer`.

The first PG18.7 attempt selected Homebrew PG17.11 after finding an empty bin
directory. Its TLS compatibility test failed the version assertion. The retry
scripts require the installed binary to report the intended version, and the
receipt verifies the running server version of each successful compatibility
and fence check.

The PG17.11 gate includes the parity-driver self-check and the set-based
enqueue wire proof. PG18.7 and PG19.0 exercise the fresh and upgrade J6 role
checks. The set-based writer installs on PostgreSQL 17 only.

The workspace command stopped at `actors_100k`. The executable printed
`100000 actors done` and remained alive until the test killed it.
`workspace_remainder.py` runs the previously unreached test binaries and
doctests. They passed with the standalone compiler performance proof and the
`e2e_performance` compile timing test excluded. The original workspace gate
remains failed. Linux AMD64 qualification is outside this receipt.

## Regenerate the receipt from retained logs

Run from the repository root. The source commit must be available in local Git.

```sh
proof=tools/durable-jobs-bench/results/2026-11-24-qualified-hot-pool
lane=target/native-qualified-pool-transfer
# For a fresh copy, first create the source worktree.
# mkdir -p "$lane"
# git worktree add --detach "$lane/source" 1e70f18d624a841740d3bbb3d0fd09b269257f68
set -o pipefail
python3 -B "$proof/local_receipt.py" "$lane" --archived "$proof/evidence/local" |
  diff -u "$proof/evidence/local-receipt.json" -
```

## Re-run the correctness commands

Use a new lane. The recorded versions are in
[`toolchain.txt`](evidence/local/toolchain.txt). Use macOS ARM64 with the
matching Rust toolchain, Homebrew LLVM 21, Homebrew PostgreSQL 17.11, OpenSSL
3, Python 3.11 or newer, and the Apple command-line tools. Put Zig 0.15.2 on
`PATH` and ensure Cargo's locked dependencies are cached. The PostgreSQL source builds require the
original archives, identified by SHA-256 in `build-postgres.sh`. The commands
below use the copies retained in the original lane.

```sh
set -eu
proof=tools/durable-jobs-bench/results/2026-11-24-qualified-hot-pool
archived="$proof/evidence/local"
lane=target/native-qualified-pool-transfer-replay
mkdir "$lane"
mkdir "$lane/postgres" "$lane/logs"
git worktree add --detach "$lane/source" 1e70f18d624a841740d3bbb3d0fd09b269257f68
cp "$archived"/{compat.sh,compat-rebuilt.sh,build-postgres.sh} "$lane/"
cp target/native-qualified-pool-transfer/postgres/postgresql-{18.7,19.0}.tar.bz2 "$lane/postgres/"
cp "$archived/toolchain.txt" "$lane/logs/"
# The historical workspace command exited before reaching the compile timing test.
sed '/^gate workspace-correctness /s/$/ --skip e2e_performance/' "$archived/gates.sh" > "$lane/gates.sh"

if ! bash "$lane/gates.sh"; then
  if grep -Eq '^workspace-correctness [1-9][0-9]*$' "$lane/logs/status"; then
    python3 -B "$proof/workspace_remainder.py" "$lane" || cat "$lane/logs/status"
  else
    exit 1
  fi
fi

for version in 18.7 19.0; do
  bash "$lane/build-postgres.sh" "$version" > "$lane/logs/build-pg-$version.log" 2>&1
done
bash "$lane/compat-rebuilt.sh"
python3 -B "$proof/local_receipt.py" "$lane"
```

`compat.sh` is retained for the historical attempt. The replay uses
`compat-rebuilt.sh` with the binaries built in its own lane.
