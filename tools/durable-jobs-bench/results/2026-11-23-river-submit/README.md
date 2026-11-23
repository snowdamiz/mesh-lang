# River submit timing setup proof

Use [READY.json](READY.json) to pin the replacement Linux AMD64 binary and
the files that implement its strict wrapper. The native large and sparse
runs are functional SETUP receipts. The matrix owner supplies the warmup and
measured series. The original binary and its strict-wrapper restage remain
available for SETUP and replay of the earlier evidence. [Original River
setup archive](../2026-11-23-river-smoke/README.md)

## Submit boundary

River checks the ordered file hashes before runner readiness. It retains
only the paths. For each submission, after readiness and idle, it stamps
`submitted_us` immediately before one `os.ReadFile` and one
`json.Unmarshal` of that batch. The bytes come from the canonical profile
generator. The large batch contains 64 jobs with 524,288-byte payloads,
and the canonical JSON file is 33,561,352 bytes. Sparse reads its 24 files
one at a time, in order, after the preceding job settles and the 250 ms
arrival gap elapses.

The strict verifier checks the receipt byte count against the canonical
file and verifies the following order on the host's epoch-microsecond clock.

```text
submitted_us <= read_finished_us <= decoded_us <= enqueue_called_us
             <= PostgreSQL producer commit <= returned_us
```

The wrapper also reconciles the request and queue IDs, payload hashes,
product transaction, two independent application writes, durable terminal
commits, and runner and database caps. A rollback has a receipt and no
PostgreSQL commit. The fault trial checks that the effect survives the
single retry.

## Residual producer work

The four arms start the submit clock before dispatching the current frozen
batch. The work after that boundary follows each producer's existing path.

- River reads and decodes the file in the producer process and calls stock
  `InsertManyTx` in the product transaction. The library marshals its
  arguments, and the adapter checks the returned job IDs before commit.
- Mesh's Python driver reads and parses the file, serializes the entire
  parsed batch into a JSON broker request, and writes it to the pipe. The
  compiled Mesh session reads and parses that request before enqueue.
- Oban's Python driver sends a small JSON request with a path. The Elixir
  broker decodes the request, reads the file, and decodes the batch with
  Jason before enqueue.
- Apalis's Python driver likewise sends the path in a small JSON request.
  The Rust broker reads the file and decodes the batch with Serde before
  enqueue.

Fixture generation, preflight hashing, runner startup, and the ready idle
window precede submission. Reads can benefit from the host page cache. The
submit-through-terminal metric includes each arm's broker transport,
marshaling, and normal enqueue path through its last durable terminal
commit. Use a fresh inventory and fresh controls and baseline for matched
submit comparisons. The earlier River binary started its clock after input
decoding.

## Replay the proof

Run from this directory with Python 3.11 or later.

```sh
shasum -a 256 -c SHA256SUMS
python3 -B replay.py | diff - replay.json
```

Replay verifies the exact staged source, both native receipts, the local
large full warmup and main run, and the sparse fault witnesses. It also
replays the local short attempt. Its raw ledger is valid, and matrix
reconciliation rejects the missing Linux RSS sample on macOS. The replay
then rejects a read timestamp before submit, an enqueue call after commit,
and a changed byte count.

To regenerate the pinned Linux binary, use Go 1.26.4 and set
`build` to an absolute scratch directory.

```sh
mkdir -p "$build/source"
tar -xzf wrapper-source.tar.gz -C "$build/source"
(cd "$build/source" && shasum -a 256 -c SHA256SUMS)
(
	cd "$build/source/tools/durable-jobs-bench/go"
	GOTOOLCHAIN=local CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOAMD64=v1 \
		go build -trimpath -mod=readonly -buildvcs=false -o "$build/river-bench" .
)
shasum -a 256 "$build/river-bench"
```

The last digest matches `artifact_sha256` in the handoff.
