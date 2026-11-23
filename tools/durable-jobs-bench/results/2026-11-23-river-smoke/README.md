# River functional smokes, November 23, 2026

The native Linux AMD64 short and sparse captures and all five final local
Darwin ARM64 profiles used PostgreSQL 17.11 over verified loopback TLS.
These captures are SETUP diagnostics with zero official timing samples.
Matched-submit timing is on HOLD.

Run the offline replay with Python 3.11 or later. The bundle contains the
source, frozen input generator, matrix verifier, and raw synthetic records
needed by replay.

```sh
cd tools/durable-jobs-bench/results/2026-11-23-river-smoke
shasum -a 256 -c SHA256SUMS
python3 -B replay.py | diff - replay.json
```

Replay regenerated all five inputs and matched the individual batch hashes.
It verified complete, ordered job IDs and frozen argument hashes, unique
producer/attempt/effect records, product and enqueue atomicity, and two
independent application commits before each job's terminal commit. The
sparse witness also showed a rolled-back product/job pair with no visible
rows, then a separate job with two attempt commits and one durable effect.
Replay rejected 21 mutations of the captured evidence, including row
duplication, missing and substituted writes, transaction aliasing, an
invalid slot count, and wrong database, Go, River, pgx, module, or artifact
identities. [Replay output](replay.json)

| Capture | Completed | Handler upper bound / slots | Connections / cap | Running rows peak | Largest terminal transaction |
| --- | ---: | ---: | ---: | ---: | ---: |
| Native short, original wrapper | 128 | 16 / 16 | 18 / 20 | 96 | 79 |
| Native short, revised wrapper | 128 | 16 / 16 | 18 / 20 | 56 | 124 |
| Native sparse, both wrappers | 24 | 1 / 1 | 4 / 5 | 1 | 1 |
| Local noop | 1,024 | 64 / 64 | 77 / 96 | 213 | 135 |
| Local slow | 512 | 64 / 64 | 67 / 68 | 133 | 118 |
| Local sparse | 24 | 1 / 1 | 4 / 5 | 1 | 1 |
| Local short | 128 | 16 / 16 | 18 / 20 | 27 | 101 |
| Local large | 64 | 64 / 64 | 73 / 96 | 64 | 8 |

River v0.51.2 uses its stock `BatchCompleter`. A job row can remain
`running` while the returned handler's terminal update waits in a batch.
Replay sums the per-process handler high-water marks, verifies each against
its allocated slots, and reconciles runner calls with durable attempt rows by
PID. Terminal XIDs may repeat across jobs with the same commit timestamp.
Producer and application write XIDs are disjoint from terminal XIDs, and
each attempt and effect has its own transaction.

## Artifact and attempt history

- [Manifest](manifest.json) and [checksums](SHA256SUMS) identify the binaries,
  pins, and archives. Local Go 1.26.4 rebuilds reproduced both final binary
  digests byte for byte. The [Linux build metadata](linux-amd64-go-build.txt)
  lists all linked versions and module sums.
- The [original source](source-original.tar.gz) retains each tested file
  byte for byte, with portable tar metadata. The [verified source](source-verified.tar.gz)
  adds raw verification to the Python wrapper and retains the same Go
  files, module pins, fixtures, and matrix verifier. The revised wrapper
  was staged under the shared native locks and passed native short and
  sparse SETUP diagnostics with the existing executable.
- The [local archive](local-raw.tar.gz) includes the five final adapter
  captures, a sparse rerun through the patched wrapper, and the earlier
  direct-binary attempts, including partial
  ledgers for the early noop and slow failures. Those builds treated
  database `running` rows as handler occupancy. Sparse's first attempt
  failed before evidence capture because the local Unix socket path was
  too long. Historical direct-binary captures are recorded with their
  original status and build digest. The final adapter captures provide
  the database preflight and capacity records used for replay.
  [Setup history](setup-history.json)
- The initial native source staging attempt uploaded the verified binary
  but failed its source checksum check because the archive layout differed
  from the paths in the manifest. The second stage passed both checksum
  checks. The original short and sparse tarballs retain the exact
  downloaded bytes. Their checksums match the original handoff.
- The archives contain synthetic run exports and source. Local PostgreSQL
  directories, certificates, credentials, SSH material, machine paths in
  setup logs, tool caches, and executables were excluded.
