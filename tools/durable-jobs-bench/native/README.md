# Native Mesh and Oban Jobs adapters

## Matrix handoff

Both adapters passed the five frozen profiles and the dense diagnostic on
the Linode. The matrix verifier reconciled 5,552 completed jobs and 11,104
ordered application commits across the 12 SETUP arms. Each trial and its
PostgreSQL postmaster used CPUs 0 and 1 of the four-CPU Ubuntu 24.04.5 host.
Use `target/linode-jobs-native/READY.json` for the two inventory entries.
The inventory includes the adapter and shared proof file digests verified
on the host.

| Engine | Build | Version | Build manifest SHA-256 |
| --- | --- | --- | --- |
| Mesh | `mesh-v1` | `3f69b150820fd01ba716d533be4f2eedea25ed27` | `a562897e895c87251018ec2111e6e61adfb80e822db3a5f19c586581cc7e7dd3` |
| Oban Basic | `oban-v2` | `2.24.2` | `b0cd27902077fe83e0b93606fe24ae541c39351c9df3c92f3f1be363ae67d37f` |

The frozen fixture calls `Jobs.start_effects`. The original `dbcb6dbc`
runtime gates its compiled worker and effect-session implementation to
GNU Linux ARM64. The native setup uses the parent task's AMD64 port and
the original frozen fixture. The builder verified the port proof's
compiler digest, rebuilt the release runtime, and compiled the fixture
at optimization level 2.

Official timing is the matrix owner's next phase. The port owner has the
next SETUP lock window for its follow-up proof. Matched-submit timing also
requires the superseding River handoff. The original predecoded River
READY remains SETUP-only.

## Run one arm

The matrix controller starts this wrapper as root with both CPUs pinned
and passes an absolute empty evidence directory. The wrapper starts
outside the root home and drops privileges before the adapter starts
PostgreSQL. The phase is supplied in `DURABLE_JOBS_PHASE`.

```sh
base=/var/lib/durable-jobs-bench/native
adapter="$base/source-v5/tools/durable-jobs-bench/native"
NATIVE_JOBS_ARTIFACTS="$base/builds/mesh-v1" \
    "$adapter/run" mesh PROFILE ABSOLUTE_EMPTY_EVIDENCE_DIRECTORY
NATIVE_JOBS_ARTIFACTS="$base/builds/oban-v2" \
    "$adapter/run" oban PROFILE ABSOLUTE_EMPTY_EVIDENCE_DIRECTORY
```

The PostgreSQL install is `/var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin`.
Each arm uses a private database on the host disk with verified loopback
TLS, fsync, synchronous commits, commit timestamps, pg_stat_statements,
and zero reserved connections. The producer and observer share one
connection. Oban uses Erlang/OTP 29.1.1 and Elixir 1.20.4, the archived
file-only resolver, and `+S 2:2 +SDcpu 1:1 +SDio 1 +A 2`. Preflight and
live runner/broker reports capture their effective settings.

| Profile | Runners | Slots | PostgreSQL cap | Jobs | Payload bytes | Hold ms | Enqueue transactions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| dense (diagnostic) | 8 | 64 | 96 | 1,024 | 1,024 | 0 | 1 |
| noop | 8 | 64 | 96 | 1,024 | 1,024 | 0 | 1 |
| slow | 1 | 64 | 68 | 512 | 16,384 | 100 | 1 |
| sparse | 1 | 1 | 5 | 24 | 1,024 | 1 | 24 |
| short | 1 | 16 | 20 | 128 | 1,024 | 1 | 1 |
| large | 8 | 64 | 96 | 64 | 524,288 | 1 | 1 |

The emitter writes `row.json` and the matrix `ledger.json` after it
matches the frozen input, enqueue IDs, durable queue state, and two
independent ordered application commits per job. The product row's
transaction ID and timestamp must agree with the enqueue receipt and
fall within the submit-to-acknowledgment window. `raw.json` retains
runner and database samples. `application-records.json` contains the
full set of attempt and effect identities and transaction IDs. The
database and generated credentials live in disposable scratch.

## Rebuild or smoke in SETUP

Use the local `linode check` command to probe SSH. It uses the dedicated
identity and persistent TOFU host file. Bundle and upload each source
attempt under a fresh ID. The compiler tree comes from the requested
commit and the fixture and proof scripts stay pinned to the base replay.

```sh
lane=tools/durable-jobs-bench/native/linode
"$lane" bundle SOURCE_ID 3f69b150820fd01ba716d533be4f2eedea25ed27
"$lane" upload SOURCE_ID
```

Run `bootstrap BUILD_ID oban` or `bootstrap BUILD_ID mesh` from that
source tree on the host. For a verified prebuilt compiler, set
`NATIVE_JOBS_MESHC`, `NATIVE_JOBS_MESHC_SHA256`, and
`NATIVE_JOBS_MESHC_REVISION` to the recorded handoff values. The Mesh
build still rebuilds the release runtime and links the frozen executable.
Both bootstrap and run acquire `/var/lib/durable-jobs-bench/native/native.lock`.
The matrix uses its own active-arm lock for dispatch.

Run a local `linode smoke SOURCE_ID BUILD_ID ENGINE PROFILE 0,1 ATTEMPT_ID`
for each native functional diagnostic. Each call retains the remote
attempt and copies its raw evidence to `target/linode-jobs-native/native-smoke`.
It uses the diagnostic phase, so the result stays outside timed scoring.

## Evidence

The versioned archive is `target/linode-jobs-native/2026-11-23/native-raw.tar.gz`.
It preserves the setup failures and retries as well as the 12 successful
native arms. The same directory holds the archive checksum, job-count
rows, build identities, and wrapper verification. The first Oban
short attempt caught a broker startup failure from the SSH working
directory. Its retry and subsequent profiles passed with the corrected
wrapper. Local macOS Oban controls also verified that the score rejects
missing work, shared XIDs, out-of-window commits, and changed payloads.
