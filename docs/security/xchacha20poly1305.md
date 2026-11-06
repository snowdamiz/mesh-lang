# Explicit extended-nonce AEAD (isolated candidate)

`Crypto.xchacha20poly1305_seal(key :: borrow AeadKey, nonce :: Bytes, associated_data :: Bytes, plaintext :: Bytes) -> Bytes!CryptoError` and the corresponding `_open` (last argument ciphertext) use XChaCha20-Poly1305 with an exact24-byte nonce. Flat `crypto_` aliases share the same contract. Both borrow the existing opaque32-byte `AeadKey`; neither exports key bytes. The caller must not reuse a nonce with the same key. Existing `aead_seal/open` remain ChaCha20-Poly1305 with12-byte nonces.

This uses the existing pinned RustCrypto chacha20poly1305 dependency; no new cryptographic implementation or dependency is introduced. AAD is limited to128KiB, plaintext to128KiB, ciphertext to128KiB+16. Combined output appends the16-byte tag. Authentication or length failure exposes no partial plaintext; temporary plaintext buffers zeroize. Existing typed key ownership/owner checks apply.

Plaintext flow treats successful sealing output as ciphertext while preserving nonce/AAD/caller-control dependencies and private failure payloads. `declassify_discriminant` may review only a bounded result tag; it cannot disclose private metadata. Opening conservatively preserves input dependencies. Local lookalike definitions must retain their own summaries.

Motivation: [Signal Double Ratchet section4.2](https://signal.org/docs/specifications/doubleratchet/#external-functions-1) requires nonrepeating stateful nonces or at least128 random bits for repeated header keys. [Libsodium's primary documentation](https://doc.libsodium.org/secret-key_cryptography/aead/chacha20-poly1305/xchacha20-poly1305_construction) documents the192-bit nonce construction and random nonce use. Provider known-answer data comes from [XChaCha draft03 appendixA.3.1](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03#appendix-A.3.1), an expired Internet-Draft, not an IETF standard.

Qualification is scoped and pending until evidence is sealed. This does not change public release/default compiler/runtime pins or claim an independent cryptographic audit.
