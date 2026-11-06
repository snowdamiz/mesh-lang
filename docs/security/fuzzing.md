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

## Blind RSA BR1

The BR1 boundary set must exercise canonical SPKI parsing, blind, finalize,
verify and signing rejection through `fuzz_crypto_boundaries`. Seed inputs
should include the public RFC 9578 Type 2 SPKI/signature/blinded-message
fixtures, a truncated SPKI, an altered PSS parameter, short and over-modulus
representatives, and modified signatures. Server calls remain target-gated;
client parsing and verification belong in every supported target build.

A checked-in corpus and a compiling harness are coverage preparation. A
release record must separately identify an executed fuzz run, its exact
revision, duration, target, sanitizer configuration, corpus and crash results.
No corpus or smoke-test result alone establishes complete cryptographic fuzz
coverage. Published test issuer private keys are public fixture material and
must never be provisioned as application credentials.
