# PostgreSQL payload storage on 64 large Mesh jobs

PostgreSQL 17.11 spent 407–448 ms more backend CPU per 64-job enqueue attempting default compression than it spent with external, uncompressed storage. Those are paired differences against the faster of two identical controls in each of four rotated blocks on macOS ARM64. All 64 canonical argument values ended up uncompressed under every policy. This was a single-client PostgreSQL component experiment.

| Payloads and column policy | Median enqueue backend CPU, ms | Median enqueue wall, ms | Stored argument bytes for 64 jobs |
| --- | ---: | ---: | ---: |
| Frozen, stock A | 615 | 744 | 33,559,808 |
| Frozen, stock B | 600 | 703 | 33,559,808 |
| Frozen, EXTERNAL | 156 | 255 | 33,559,808 |
| Frozen, LZ4 | 188 | 296 | 33,559,808 |
| Repeated-character control, stock A | 129 | 158 | 389,739 |
| Repeated-character control, stock B | 127 | 162 | 389,739 |
| Repeated-character control, EXTERNAL | 180 | 322 | 33,559,808 |
| Repeated-character control, LZ4 | 58 | 104 | 137,726 |

The second series used the same 64 job specifications and 512 KiB payload length, with each payload filled with one repeated character. EXTERNAL grew the stored arguments about 86-fold and cost more producer CPU in all four rotations. LZ4 reduced CPU on both inputs in this PostgreSQL build. The frozen native matrix builds PostgreSQL with OpenSSL and its original compression configuration. Its EXTERNAL comparison works on that installation. Storage choices are per column and also affect future writes of other jobs in the same table.

Each arm used a fresh database, the production Jobs schema, one producer transaction, request keys and the normal per-batch savepoint and namespace admission SQL. Eight successive committed claims returned eight fully reconstructed contexts each. The checker compared each payload against its input, then settled all 64 first attempts. The component excludes TLS, concurrent workers, and application writes. Measured wall time includes client calls over a private Unix socket. Backend CPU uses a separately calibrated Mach clock. Fresh full-queue results use the original native worker and verifier. [Native SETUP receipts](../2026-11-23-payload-storage-mesh-setup/README.md)

### Sampled compression experiment

A PostgreSQL module sampled eight 4 KiB regions before deciding whether to try PGLZ on each large JSONB value. In four rotated blocks with the original frozen payloads, the enqueue path using the module consumed 197 ms median backend CPU per 64 jobs. The stock controls consumed 632 and 634 ms. For the repeated-character input the respective medians were 131, 134 and 135 ms. A version applied by a table trigger gave the same direction on both fixtures. The initial version declared the function immutable and left five of the 64 repeated-character payloads uncompressed.

A nonuniform 512 KiB fixture exposed the limit of the sampling rule. Its sampled regions looked incompressible, while most of its bytes were repeated text. The module stored 524,307 bytes, stock PGLZ stored 54,555 and LZ4 stored 49,185. All four storage methods round-tripped the same input hash. The experiment ended with this counterexample. The native tuning series uses the explicit PostgreSQL column policy.

The [component archive](adaptive-screen.tar.gz) includes the three 32-arm series, mixed-payload results and diagnostic module sources. Its SHA-256 is `1b4449f464dbcba99604355dfc375ff7efaa141a27d5eb408c141e9eb6ef32af`.

## Reproduction

The compiled Mesh [transaction proof](transaction-local/main.mpl) enqueues jobs and writes a business row on the same producer connection. On PostgreSQL 17.11, `SET LOCAL` selected LZ4 for the committed job. The connection returned to PGLZ after both commit and rollback. The rolled-back job and business row were absent. A repeat migration preserved both the owner's EXTERNAL policy and the later return to EXTENDED, as well as the three previously committed tuple versions and codecs. [Recorded results](transaction-local/receipt.json)

With the repository's release runtime archive, compiler and an LZ4-enabled PostgreSQL 17.11 installation, reproduce this check from the repository root. Use a new output directory for each run.

```sh
proof=tools/durable-jobs-bench/results/2026-11-23-large-storage-component/transaction-local
out=$(mktemp -d "$PWD/target/jobs-storage-transaction.XXXXXX")
mkdir "$out/program"
cp "$proof/main.mpl" "$out/program/"
target/debug/meshc build "$out/program" --opt-level 2 --output "$out/producer"
python3 -B "$proof/prove.py" "$out/producer" "$out/evidence"
```

Set `MESH_RT_LIB_PATH` to select the runtime archive and `PG_BIN_DIR` to select the PostgreSQL bin directory. The driver creates a disposable TLS cluster under `target/`.

From the repository root, replay both four-block reductions and verify the hashes of the measurements and executed support code.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-large-storage-component/replay.py
```

On macOS with PostgreSQL 17.11 built with LZ4, set `PG_BIN_DIR` to its bin directory and append `--run target/storage-fresh` to rerun both experiments into a new directory. A fresh run rebuilds its private databases and records every arm. The archive also includes two initial setup failures, one for a Unix socket path length and one for a diagnostic client that required an idle connection inside a transaction. Both occurred before collecting a full block. [Per-block evidence and captured source](evidence.tar.gz) · [Paired differences and medians](summary.json)
