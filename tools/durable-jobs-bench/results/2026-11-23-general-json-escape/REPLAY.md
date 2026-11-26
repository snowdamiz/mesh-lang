# Replay the Json.encode experiment

Extract `evidence.tar.gz` under an empty directory. The archive contains the
results, source files, and patches. The archive includes source for the
frozen broker and its FFI shim. It does not include executable files.

Use `results/fixtures/manifest.json` for input hashes and fixture settings.
The JSONL files record each timed process. Summary files give minimum,
median, and maximum values in that order. In each series, A and B use
the same release executable. Verified output files contain the A bytes
and the correctness manifests give the output digests for every arm.

To re-run the macOS probes in the original checkout, use the archived
`replay/patches/*.patch` files against
`dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17`. Build `mesh-rt` with
Rust 1.99.0 and `cargo build --release --locked -p mesh-rt` for each arm.
Save each static runtime as `target/general-json-escape/bins/ARM.a`.
Use `stock.a` for the unmodified build. Create `logs` and `tmp` under
`target/general-json-escape`. The compiler and broker manifest in
`replay/frozen-local-manifest.json` pin the already-frozen macOS inputs
under `target/linode-jobs-stepchange/direct-producer-v3`.

From this results directory, run the component comparison with the
matched compiler, frozen broker source, and rebuilt runtime libraries.

```sh
python3 -B measure.py fixtures
python3 -B measure.py link --arms stock skip
python3 -B measure.py prove --arms A skip B
python3 -B measure.py measure --arms A skip B --rotations 8
python3 -B measure.py summarize
```

With PostgreSQL 17.11 and the frozen adapter available, link the
`blocks` runtime and prove its broker chunks. Run the local timing
comparison against fresh disposable clusters.

```sh
python3 -B measure.py link --arms blocks
python3 -B measure.py prove --arms A blocks B
PG_BIN_DIR=/opt/homebrew/opt/postgresql@17/bin \
  python3 -B local-pg.py --arms A blocks B --profiles broker-large \
  --rotations 6 --label pg-blocks
```

The archive is readable on other platforms. Repeat timings on current
hardware with matched binaries and compare against duplicate controls
from the same run.
