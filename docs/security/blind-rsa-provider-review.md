# BR1 provider review

Status: implementation and source review, not an external audit or release
approval. BR1 activation still requires the release revision's cryptographic
release gates, target execution, dependency evidence, and specialist review.

## Exact capability and dependency selection

The public `Crypto.blind_rsa_*` API implements BR1,
`RSABSSA-SHA384-PSS-Deterministic` from RFC 9474 as used by RFC 9578 Type 2:
RSA-2048, exponent 65537, SHA-384, MGF1-SHA-384, 48-byte PSS salt, 256-byte
representatives/signatures, and messages of at most 65,536 bytes. “Deterministic”
names the RFC preparation variant; the PSS salt and blinding factor are random.
The public key is the exact 342-byte RFC 9578 SPKI template, including explicit
PSS parameters. The decoder rejects other OIDs, parameters, lengths, exponents,
DER templates, even moduli, and moduli shorter than 2048 bits. It does not prove
that a public modulus is the product of two appropriately generated primes.

| Dependency | Selection | Role | License |
|---|---|---|---|
| `crypto-bigint =0.6.1` | defaults off; `zeroize` enabled | Fixed-width client Montgomery multiplication, exponentiation, constant-time inversion | Apache-2.0 OR MIT |
| `ring =0.17.14` | existing provider, now exact-pinned | CSPRNG, SHA-256/SHA-384 and `RSA_PSS_2048_8192_SHA384` verification | Apache-2.0 AND ISC |
| `aws-lc-sys =0.46.0` | defaults off; no `all-bindings`; Linux/macOS only | Validated PKCS#8 import, key generation, blinded raw issuer transform | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) |

Relevant new build dependencies are `cmake 0.1.58` (MIT OR Apache-2.0),
`dunce 1.0.5` (CC0-1.0 OR MIT-0 OR Apache-2.0), and `fs_extra 1.3.0` (MIT).
Client arithmetic also uses the existing `subtle 2.6.1` (BSD-3-Clause) and
`zeroize 1.8.2` (Apache-2.0 OR MIT) selected by the lockfile. These permissive
licenses are compatible with this repository; redistribution must retain the
providers' notices. The full graph and notices remain SBOM/license gates.

The existing ring provider verifies PSS but does not expose the required raw
private RSA primitive. `crypto-bigint` supplies client arithmetic with reviewed
constant-time interfaces; it does not implement private-key signing here.
AWS-LC provides the raw private transform and its implementation hardening.
No pure-Rust `rsa` crate is added. OpenSSL CLI comparisons are test tooling,
not a linked runtime dependency, fallback, or runtime provider selector.

Primary references:
[RFC 9474](https://www.rfc-editor.org/rfc/rfc9474.html),
[RFC 9578](https://www.rfc-editor.org/rfc/rfc9578.html),
[crypto-bigint 0.6.1 documentation](https://docs.rs/crypto-bigint/0.6.1/crypto_bigint/),
[AWS-LC RSA API](https://aws.github.io/aws-lc/rsa.h.html),
[aws-lc-sys 0.46.0](https://docs.rs/aws-lc-sys/0.46.0/aws_lc_sys/), and
[ring verification parameters](https://docs.rs/ring/0.17.14/ring/signature/).
The evidence is tied to the exact crate sources/checksums in `Cargo.lock`;
upstream descriptions of audits do not establish that this integration or pin
has been externally audited.

## Client arithmetic and timing boundaries

The client constructs EMSA-PSS with SHA-384 and fixed public layout bounds,
then checks that the encoded representative is coprime to the modulus using
the constant-time inversion path required by RFC 9474 section 4.2. A nonunit
encoded representative is `InvalidPublicKey` and produces no blinded output.
Blinding samples 128 fixed-width random candidates, uses `subtle` selection to
select the first integer in `1..n`, and performs one constant-time inversion.
For a validated full-width modulus, the chance that all 128 samples are outside
that interval is at most approximately 2^-128. Entropy failure, no accepted
sample, or a noninvertible selected factor returns an error without a state
resource. For a properly generated RSA modulus, the latter event is negligible.
A malicious non-RSA public modulus can cause inversion to fail; import does not
make a claim about an issuer's real-world identity or key-generation process.

`MontyForm`/`MontyParams` and `pow_bounded_exp` operate with fixed 2048-bit
operands and the public exponent bound 17. The inversion call is
`Uint::inv_odd_mod`, not an `_vartime` inverse. Inspection of 0.6.1's
`modular/safegcd.rs` and `uint/inv_mod.rs` shows the constant-time divsteps
path. Its iteration bound depends on `max(bits(n), bits(r))`; `n` is exactly
2048 bits and `r < n`, so that bound is fixed for BR1. Its final success bit
is tested only after completing that arithmetic. Secret selection, comparisons,
and message/key binding equality use `subtle`. Public lengths, parsing, and
error selection can branch normally. No private exponent enters this client
arithmetic path.

The state contains exactly 336 secret bytes: the 256-byte big-endian inverse,
SHA-256 of the canonical SPKI, and SHA-384 of the message. Finalization requires
both bindings, rejects out-of-range values, unblinds with Montgomery
multiplication, then verifies the resulting PSS signature with ring before
returning it. Failed verification does not expose the candidate signature.

## Issuer provider, faults, and cleanup

Only `cfg(any(target_os = "linux", target_os = "macos"))` depends on or imports
AWS-LC. Client SPKI import, blind, finalize, verify, and blinding-state storage
remain available on iOS, Android, and Windows. Server key generation/import,
public derivation from a server secret, signing, and server-key storage fail
with the appended `UnsupportedTarget` error there. No alternate issuer provider
is compiled as a fallback. A target archive build establishes compilation and
provider exclusion, not successful execution of its vectors.

Private input is bounded PKCS#8 DER, with complete parser consumption. Import
requires a private RSA key, exactly 2048 bits, public exponent 65537, and
`RSA_check_key == 1`. Each loaded/generated key checks
`RSA_blinding_on == 1` and rejects `RSA_FLAG_NO_BLINDING`. The implementation
calls only `RSA_sign_raw(..., RSA_NO_PADDING)` for nontrivial valid public
representatives and checks the exact 256-byte output length.

Source inspection of bundled `crypto/fipsmodule/rsa/rsa_impl.c` in the selected
crate confirms blinding unless disabled, fixed-time private modular
exponentiation/CRT paths, and its internal RSA fault checks. AWS-LC rejects the
public representatives 0, 1, and n-1 before its transform. BR1 handles those
three public inputs with their mathematically exact identity outputs **after**
private-key validation. Every result, including those identities, goes through
an independent constant-time public exponentiation and input comparison before
it can be returned. This special case does not accept an invalid private key
or bypass the output fault check. Range n and above are rejected.

Rust RAII frees RSA and EVP objects on every path. PKCS#8 marshal buffers are
explicitly cleansed before CBB cleanup, including error paths. Bundled
`crypto/fipsmodule/rsa/rsa.c` releases all RSA BIGNUMs; `BN_free` delegates to
`OPENSSL_free`, whose default allocator in `crypto/mem.c` cleanses the complete
allocation before releasing it. This review assumes the bundled default
allocator; the Mesh integration does not install AWS-LC allocator callbacks.
Controlled Rust key/state, PSS, inverse, and intermediate arithmetic buffers
use `Zeroizing`. The actor resource table zeroizes key/state allocations on
consumption, explicit destruction, failed retyping, and actor exit.
These facts do not guarantee erasure of every compiler-generated stack spill,
register copy, third-party internal temporary, crash dump, or OS memory copy.

## Resource and storage contract

Resource kinds 9 and 10 are `BlindRsaSecretKey` and
`BlindRsaBlindingState`. They use the existing generational actor-owned table,
quota accounting, kind/owner checks, and generic destructor. There is no
resource-to-`Bytes` export. Import consumes valid owned `SecretBytes` on success
or failure; finalize consumes valid owned state before validating its other
arguments. Aliases therefore cannot repeat a finalize attempt after either an
error or success. A stale/wrong-owner handle fails without exposing material.

Storage purposes 18 and 19 bind the key and blinding state respectively to
existing authenticated storage context/account/device/snapshot fields. Both
require a zero session ID. Server-key plaintext is bounded and validated as a
BR1 private key before sealing and after authenticated decryption. State
plaintext must be exactly 336 bytes. Context/type/session/tamper failures do not
produce a resource. Counter reservation and nonce semantics are unchanged.
State persistence lets a client save recovery state before paid issuance.
Repeatedly restoring the same state and finalizing the same response produces
the same signature and token nullifier; storage restoration is not a mechanism
for producing a second spendable value. Applications must bind their own
protocol origin and issuance identifiers in the signed message/context.

## Evidence and remaining gates

Focused runtime tests cover all five RFC 9578 exact blinded requests, raw
issuer signatures, unblinded signatures and verification; all four RFC 9474
encoding/unblinding vectors through test-only 4096-bit helpers; fresh generated
and imported-key round trips; empty/64-KiB messages; malformed SPKI/PKCS#8,
range/length/entropy failures; trivial public representatives; output fault
checking; moved-resource errors; and authenticated key/state storage recovery.
The public compiled API, OpenSSL differential, JIT, optimization, and mobile
build proofs are separate evidence lanes. The provider fuzz boundary includes
BR1 parser/verification/finalization plus sparse private-parser/blinding paths.
The ignored release timing test compares low/random public representatives
with a rotated identical-input control and emits schema-2 evidence. A timing
pass is supporting host evidence, not a constant-time proof or external audit.

The initial 2026-10-20 RustSec audit found seven existing TLS dependency
advisories. The lockfile now selects rustls 0.23.46, rustls-webpki 0.103.16,
and webpki-roots 1.0.10. The old direct 0.26.11 webpki-roots compatibility
wrapper has no patched 0.26 release and was removed in favor of its compatible
1.0 export. The corresponding official security releases are
[rustls 0.23.46](https://github.com/rustls/rustls/releases/tag/v%2F0.23.46),
[webpki 0.103.16](https://github.com/rustls/webpki/releases/tag/v%2F0.103.16), and
[webpki-roots 1.0.10](https://github.com/rustls/webpki-roots/releases/tag/v%2F1.0.10).
The test-only Solana SDK oracle migrated from bincode to wincode 0.6.2,
with lookup-table SDK 3.2.0 and slot-hashes 3.2.0, retaining independent fixed
legacy wire vectors. The final local audit reports zero vulnerabilities and
zero warnings, without ignores. Record the audit again from the release
revision; local findings are not a permanent assurance about a dependency.
No passing local test substitutes for target execution, SBOM/licenses,
reproducibility, external specialist review, or explicit release activation.
