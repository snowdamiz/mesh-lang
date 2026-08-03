# Fuzzing

Mesh uses `cargo-fuzz`/libFuzzer for native public boundaries. Install the
pinned release tool and a nightly Rust toolchain, then list or run targets:

```bash
rustup toolchain install nightly
cargo install cargo-fuzz --version 0.13.2 --locked
export PATH="$(dirname "$(rustup which --toolchain nightly cargo)"):$PATH"
cargo fuzz list
cargo fuzz run byte_operations
```

`bash scripts/fuzz-smoke.sh OUTPUT_DIRECTORY [SECONDS_PER_TARGET]` runs the
release smoke set and writes machine-readable evidence plus one log per target.
It covers GC-backed byte operations, the production crypto provider, runtime
protocol/routing/WebSocket decoders, and the lexer/parser.

This is not yet complete cryptographic-release fuzz evidence. Generated Mesh
messenger codecs, storage blobs, ratchet and handshake messages, attachments,
and transparency proofs still need coverage-guided harnesses through their
actual public Mesh entrypoints. Release records must keep that limitation until
those targets run from the release revision.
