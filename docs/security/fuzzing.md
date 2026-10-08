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
protocol/routing/WebSocket decoders, and the lexer/parser. The crypto target
(`crypto_provider`) also covers blind RSA: every input is parsed as an SPKI,
and inputs whose first byte selects it (`& 0x1f == 2`) are blinded and
finalized under the RFC 9578 issuer key, verified, checked as a blinded
message, and (on Linux and macOS) imported as PKCS#8 and signed with a
generated issuer key. The `fuzz/corpus/crypto_provider/blind-rsa-*` seeds
start each of these paths; like each target's `basic` seed they are tracked
with `git add -f`, because `cargo fuzz` writes its own finds into the ignored
corpus directories.

This is not yet complete cryptographic-release fuzz evidence. Generated Mesh
messenger codecs, storage blobs, ratchet and handshake messages, attachments,
and transparency proofs still need coverage-guided harnesses through their
actual public Mesh entrypoints. Release records must keep that limitation until
those targets run from the release revision.
