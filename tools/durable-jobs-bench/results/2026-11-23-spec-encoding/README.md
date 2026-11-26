# CPU cost of preparing Jobs enqueue specs

This local component runs the production Rust spec decoder, validation, canonical JSON serialization and request-key fingerprint. It compares selecting the deserializer from the first JSON token, moving the argument and metadata trees while building the canonical object, and applying both changes. All five modes produced identical normalized specs, keys and fingerprints on all four workloads.

| Batch | Stock A | Both changes | Stock B |
| --- | ---: | ---: | ---: |
| 1,024 dense jobs | 6.78 ms | 6.08 ms | 6.49 ms |
| 128 short jobs | 0.87 ms | 0.79 ms | 0.88 ms |
| 64 large jobs | 79.18 ms | 80.69 ms | 81.56 ms |
| 24 sparse jobs | 0.20 ms | 0.20 ms | 0.21 ms |

Times are median subject process CPU per workload across five order rotations on macOS ARM64. The dense and short combined variants used less CPU than both controls in each rotation. The large combined variant did so once. The driver reads an input file before the timed loop and divides full process CPU by the number of repetitions. The component ends before PostgreSQL or network I/O. With a sub-millisecond dense saving and overlapping large results, the experiment ended before product integration or native queue timing. Compatibility on malformed input remains untested. [Arm data and checked source](component.tar.gz) · [Medians and build identity](result.json)

The source archive includes both extracted implementations, the harness, the SHA-256s of each input and output, and all 100 timed rows. Its SHA-256 is `d3b1a330e021048a2c28a1fa496d2d37e64c439775e768d3307e65fbaf426a22`. For a fresh run, extract it under `target/`, compile `bench.rs` with Rust 1.99 and the release dependency paths printed in `build.py`, then run `python3 -B run.py replay` from the extracted directory.
