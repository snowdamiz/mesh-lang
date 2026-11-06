# ML-KEM-768 provider boundary

The runtime uses `libcrux-ml-kem =0.0.11` (Apache-2.0), published from
[commit 42d68bd49b244b09bd02626c6ffa95e54dc64f91](https://github.com/celabshq/libcrux/tree/42d68bd49b244b09bd02626c6ffa95e54dc64f91).
Default features are disabled and only `mlkem768` is enabled. The Mesh adapter
calls `mlkem768::portable` explicitly; the portable implementation uses
`libcrux_sha3::portable`. The upstream build may compile other architecture
implementations, but the adapter does not select them or an alternate provider.

The public profile is unchanged: FIPS 203 ML-KEM-768, seed `d || z` of 64 bytes,
encapsulation key 1184 bytes, ciphertext 1088 bytes and shared secret 32 bytes.
Canonical public-key validation precedes entropy generation. All full-length
ciphertexts decapsulate to a shared secret, including implicit rejection for
invalid ciphertexts; the API does not expose a ciphertext-validity oracle.
Existing outer ABI length, ownership and stale-handle checks remain in place.
Storage retains the seed under the existing ML-KEM purpose/type, never an
expanded private key. No key, storage, resource, ABI or wire version changes.

Mesh obtains all production randomness from its existing provider. The libcrux
`rand` feature is disabled. Its mandatory `libcrux-traits 0.0.9` dependency still
includes default-features-off `rand 0.10.3` for traits. Proof-only configurations
also resolve packages in Cargo.lock; those are distinct from the normal target
dependency graph. The lockfile and target-specific feature tree are the
authoritative version/activation inventory.

The published `core-models 0.0.7` proof-only package still requests hax-lib
`0.3.7`, unlike the ML-KEM provider's pinned upstream workspace, which uses
`0.4.1`. Mesh's [vendored metadata patch](../../vendor/core-models/PATCH.md)
preserves every published Rust/F* body and aligns only that requirement with
the pinned workspace. The lock resolves `0.4.2`; the stale unmaintained
proc-macro-error2 chain is removed. The zero-warning audit policy is unchanged.
Standalone model Rust tests are distinct from hax extraction/F* verification,
which this qualification does not rerun.

The returned upstream packed private-key type has neither `Zeroize` nor a
zeroizing `Drop`. The adapter therefore holds each expanded private buffer in a
non-Clone owner whose destructor wipes all 2400 bytes on normal return and
unwind. Adapter-owned seed/randomness/shared-secret copies use `Zeroizing`;
retained seeds and shared secrets continue through the actor-owned resource
path. This is not a guarantee that every compiler-created stack/register copy
or every transient buffer inside upstream code is erased. Forced process
termination, abort, crash dumps and swap remain outside this destructor claim.

[Upstream's verification table at the pinned commit](https://github.com/celabshq/libcrux/blob/42d68bd49b244b09bd02626c6ffa95e54dc64f91/libcrux-ml-kem/proofs/verification_status.md)
describes itself as a rough guide. It lists complete portable arithmetic,
NTT, serialization, compression and sampling entries, but incomplete generic
ML-KEM components and incomplete NEON entries. It does not establish complete
formal verification of the library, this adapter, Rust toolchain, entropy,
ownership, erasure, platform or application integration. No such claim follows
from using this crate.

The runtime conformance tests execute NIST ACVP keyGen FIPS203 `tcId 26`, packed
coefficient rejection, validation/entropy error priority, deterministic
implicit rejection and expanded-private erasure. Existing actor resource,
storage and public Mesh API tests remain required. The portable round trip
also runs on a stack sized with the runtime's actual `DEFAULT_STACK_SIZE`.

Run `bash scripts/verify-mlkem-openssl.sh` with OpenSSL >=3.5 available (or set
`MESH_OPENSSL_BIN`). This explicit external-tool gate checks exact seed-derived
public keys, deterministic encapsulation ciphertexts and both accepted and
implicit-rejection shared secrets for three public fixture seeds. The external
test is ignored in the ordinary runtime suite, and the script requires exactly
one executed successful test. No unavailable-tool skip counts as a pass.
OpenSSL is test tooling, never a runtime dependency or fallback. Public fixed
seeds and deterministic encapsulation entropy exist only in test code; the
production API gains no deterministic-entropy selector.

Host vectors and destructor tests do not prove mobile execution, integration
performance, independent constant-time behavior or release readiness. Actual
Morse hybrid/renewal dogfood, the advertised native matrix, the candidate's
audit/SBOM/reproducibility, fuzzing, and independent review remain separate
qualification gates.
