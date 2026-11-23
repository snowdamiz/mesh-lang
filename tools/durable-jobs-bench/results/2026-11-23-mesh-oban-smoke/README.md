# Replay the Mesh and Oban SETUP evidence

The original Ubuntu 24.04.5 x86-64 runs completed 5,552 jobs across 12
SETUP arms. Each job recorded two independent, ordered application commits.
Both engines ran the five frozen profiles and the dense diagnostic with
the runner and PostgreSQL pinned to CPUs 0 and 1. The runs contribute zero
samples to the timing matrix.

## Replay from Git

Run from this directory with Python 3.11 or newer and a Git checkout
containing the pinned revisions.

```sh
shasum -a 256 -c SHA256SUMS
python3 -B replay.py | diff - replay.json
```

Replay runs offline with the matrix verifier from
`1406e449cf2f24f3363369d8cb3bff33c14ba7c6` and the tracked frozen input
generator. It checks payloads, enqueue receipts, terminal rows, application
XIDs, CPU affinity, and PostgreSQL 17.11 TLS sessions. Dense keeps its
original diagnostic label and uses the no-op workload for reconciliation.
The proof replay checks the saved Linux logs and syscall trace. It requires
no access to the original host or any files under `target/`.

## Four-way baseline pins

The archived matrix inventory contains the exact commands and artifact
identities for all four engines. Mesh and Oban use the SHA-256 of their
native build manifests. River and Apalis use the SHA-256 of their native
executables. Replay checks the staged wrappers and receipts against those
entries. [Baseline inventory](../2026-11-23-baseline/inventory.json)

| Engine | Version | Preserved handoff and manifest |
| --- | --- | --- |
| Mesh | `3f69b150820fd01ba716d533be4f2eedea25ed27` | [READY.json](READY.json), [mesh.json](mesh.json) |
| Oban Basic | `2.24.2` | [READY.json](READY.json), [oban.json](oban.json) |
| River | `0.51.2`, pgx `5.13.0`, Go `1.26.4` | [Matched-submit READY.json](../2026-11-23-river-submit/READY.json), [River rebuild and replay](../2026-11-23-river-submit/README.md) |
| Apalis | `1.2.0`, apalis-postgres `1.2.0` | [Apalis receipt](../2026-11-23-native/apalis.json), [Apalis rebuild and replay](../2026-11-23-apalis-smoke/README.md) |

The exact Apalis build manifest is `adapter/build.json` inside the
Apalis archive. The baseline uses the matched-submit River handoff. Its
earlier predecoded build remains in the original River SETUP bundle.
[Apalis archive](../2026-11-23-apalis-smoke/sparse-trace-off-v1.tar.gz)
[Original River evidence](../2026-11-23-river-smoke/README.md)

## Recreate the native source and build

The committed [native adapter](../../native/README.md) has the commands
for a fresh SETUP build and diagnostic. Use `READY.json` in this
directory for the preserved Mesh and Oban entries. The original absolute
host paths identify the prior deployment. A fresh host needs its own
toolchain, build directories, and inventory.

To stage a local source bundle from the checkout root, run the following
command with a fresh source ID. It performs no SSH or build.

```sh
tools/durable-jobs-bench/native/linode bundle mesh-oban-rebuild \
    3f69b150820fd01ba716d533be4f2eedea25ed27
```

The command writes to `target/linode-jobs-native/bundles/mesh-oban-rebuild/`.
The uploaded `source-v5` used the Mesh compiler and runtime from
`3f69b150` and the frozen fixture from `dbcb6dbc`. To verify every
file against the original snapshot, restore the staged README into the
generated source tree. The current native README records the later
handoff. Archive metadata can differ across hosts. Compare file hashes
using the following commands from the checkout root.

```sh
evidence="$PWD/tools/durable-jobs-bench/results/2026-11-23-mesh-oban-smoke"
(
    cd target/linode-jobs-native/bundles/mesh-oban-rebuild/source
    tar -xOzf "$evidence/staged-source-overlays.tar.gz" v5/README.md \
        > tools/durable-jobs-bench/native/README.md
    shasum -a 256 -c "$evidence/source-v5.sha256"
)
```

The original Mesh release fixture used the AMD64 proof's dev compiler
with Rust 1.99.0 and LLVM 21.1.8. The builder rebuilt the release runtime
and linked the frozen fixture at optimization level 2. The proof owner
recorded the compiler build and LLD flags in
`amd64/prove-on-host-lto0.sh`. The original Oban artifact was built from
`source-v2` with OTP 29.1.1, Elixir 1.20.4, and the frozen Mix lockfile.
To reconstruct the Oban build inputs from Git, use the same bundle
command with a fresh ID and revision
`dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17`. In the generated tree,
restore the `v2/` files from the overlays archive into
`tools/durable-jobs-bench/native/` before building Oban. Use the
current launcher for diagnostics. The v4 overlay records the interim
staging used for the initial Oban retry. The stock baseline used the v5
launcher for both engines.

The archive records the original SHA-256 values for the Mesh compiler,
runtime archive, fixture executable, both build manifests, Oban BEAM
modules, and shared services. It contains the build recipes and checksum
records. A fresh build must verify its own binaries and supply its own
manifest digest. [Setup history](setup-history.tar.gz)

## Setup failures and AMD64 proof

- `oban-v1` stopped while unpacking PostgreSQL. The checksum-verified
  `oban-v2` bootstrap completed. The original bootstrap failure and the
  successful installation's service hashes are in `setup-history.tar.gz`.
- `oban-short-01` reached broker startup and exited with an Erlang logger
  error. The owner traced it to the inherited SSH working directory.
  The corrected launcher changed directory before dropping privileges,
  and `oban-short-02` passed.
- `oban-dense-01` and `oban-dense-02` exited 1 before the adapter wrote
  any evidence. The original logs are empty. The owner recorded native
  lock contention, and the wrapper uses a nonblocking `flock`.
  `oban-dense-03` passed. The separate lock-wait and PostgreSQL preflight
  failure and retry logs are in `setup-history.tar.gz`.
- The historical `dbcb6dbc` shell proof refused Linux AMD64 with exit 2.
  An earlier compiled control picked up the port's runtime archive and
  is invalid as a historical control. The corrected run pinned both the
  original compiler and runtime. Its compiled Jobs guard reported the
  GNU ARM64 requirement and the proof exited 1.
- Linux polling and LISTEN with THP disabled passed on the port. The
  proofs recorded a rejected bootstrap, reuse of the same verified warm
  backend, two commits, timeout reaping, retry after interruption, and
  worker exit on parent EOF. The original syscall trace records a
  consuming `wait4` for the exact child with status 171 and the parent-EOF
  worker exit with status 173. The separate PostgreSQL compatibility
  run used an isolated 17.11 copy with `pg_trgm` and `pgcrypto`. Its log
  records TLS, a refused wrong password, 85 Jobs runtime tests, and three
  compiled durable Jobs tests. [Proof logs](native-proof-logs.tar.gz)
  [Syscall trace](native-wait-trace.tar.gz) [Exact wait](exact-wait.txt)

## Evidence retained from the originals

`native-smokes.tar.gz` contains selected original member bytes, including
the ledgers, independent application rows, build identities in the run
rows, topology, TLS samples, failed attempt receipts, and broker failure
logs. `originals.json` records the owner archive and per-attempt
checksums plus the retained member hashes. The proof logs and syscall
trace are byte-for-byte copies of the original archives.

The full smoke exports, verbose installation logs, macOS controls, and
source tarballs remain under the ignored `target/linode-jobs-native/`.
If the owner files are present, rerun the allowlisted selection and verify
its output against this bundle from the checkout root.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-mesh-oban-smoke/pack.py \
    target/linode-jobs-native
```
