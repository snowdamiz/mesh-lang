# AEAD input bounds

The public ChaCha20-Poly1305 and XChaCha20-Poly1305 AEAD APIs accept zero
through 131,072 plaintext bytes and zero through 131,072 associated-data bytes.
Both require a 32-byte resource-owned key. ChaCha20-Poly1305 requires a 12-byte
nonce; XChaCha20-Poly1305 requires a 24-byte nonce. Opening accepts ciphertext through
131,088 bytes, including the 16-byte tag. A shorter-than-tag ciphertext or an
invalid tag returns an authentication error without plaintext.

The runtime validates these public lengths before looking up a key or calling
the provider. The shared provider repeats the checks and clears its working
output on authentication or length failure. Runtime and provider use the same
AEAD size constants. Slice lengths and tag offsets remain `usize`; no narrowing
conversion or platform-specific implementation truncates them.

The larger AEAD bound permits a bounded application payload plus its framing
to be sealed as one message. It does not increase other cryptographic inputs:
HMAC, signing, HPKE plaintext and associated data, secret resources and storage
wrapping keep their existing limits. HPKE's maximum sealed value is explicitly
65,584 bytes, independently of the AEAD limit. No algorithm, nonce format, tag
format, provider selection or dependency changes with this bound.

The same default RustCrypto provider methods serve all supported targets.
There is no target fallback. Callers remain responsible for nonce uniqueness
under a key and for authenticated protocol framing; a size bound establishes
neither of those properties.

The focused regression covers 65,536, 65,537, 65,606 and 131,072 bytes, verifies
round trips and tamper rejection, rejects the next plaintext/ciphertext length,
and checks the independent HPKE and HMAC limits through compiled public Mesh
at optimization levels 0 and 2. Associated-data regression tests cover both
algorithms at 65,536, 65,537 and 131,072 bytes, authenticate the final byte,
reject 131,073 bytes, and preserve the borrowed key after failed operations. The messenger additionally tests
its existing maximum sealed-delivery frame through the public API. These tests
are conformance evidence, not a cryptographic review or release approval.
Cross-platform builds and execution must follow the existing
[cryptographic release gates](cryptographic-release-gates.md) at the exact
release candidate.
