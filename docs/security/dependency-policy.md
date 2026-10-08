# Cryptographic Dependency Policy

Status: mandatory policy for adding, updating, or selecting dependencies used
by Mesh cryptography and secret handling.

This policy exists because a supply-chain attacker is in the messenger threat
model. A contributor should use it before changing a cryptographic provider,
constant-time helper, randomness source, key-memory implementation, or related
transitive dependency.

## Selection rules

- Prefer an already reviewed provider that satisfies the selected profile over
  adding another implementation of the same primitive.
- Cryptographic algorithms remain behind stable, generic Mesh APIs. The
  messenger must not add a private Rust protocol or cryptographic crate.
- The default provider is explicit and pinned.
- A deterministic test provider is compiled only in tests and cannot be
  selected at runtime in production.
- The implementation must provide the exact algorithm and behavior named by
  the versioned profile. Silent fallback or opportunistic substitution is
  prohibited.
- Unsupported targets and algorithms fail clearly.
- Secure equality and selection use a reviewed constant-time dependency, not
  a custom loop.
- Dependency versions and licenses are reviewed before acceptance.

No dependency is accepted merely because it is already transitive. If Mesh
relies on its security behavior directly, that reliance must be explicit and
covered by these gates.

## Change review

An addition or update must document:

- The Mesh capability and profile requirement it satisfies.
- Why the accepted provider set does not already satisfy that requirement.
- The exact direct and relevant transitive versions.
- Supported Mesh targets and any target-specific implementation differences.
- License compatibility and the disposition of applicable security findings.
- Whether it handles secret memory, performs allocation, or exposes variable-
  time operations.
- Any change to vectors, wire compatibility, domain separation, or downgrade
  behavior.

Before merge, the public Mesh API must pass known-answer, negative, bounds,
fuzz, and advertised-target tests with the candidate dependency. A provider
change also requires differential testing against an independent
implementation.

## Supply-chain evidence

Cryptographic release evidence must include:

- A committed lockfile resolving exact versions.
- A dependency audit with every applicable finding resolved or explicitly
  reviewed.
- A software bill of materials containing direct and transitive dependencies.
- License review for the resolved dependency graph.
- A reproducible-build check for advertised artifacts.

Generated evidence must come from the release revision. A checksum proves
artifact integrity, not that a dependency is secure or appropriate.

## Updates and profile changes

Security updates should retain the profile's observable algorithm and wire
behavior when possible. If an update changes an algorithm, encoding,
parameter, domain label, target behavior, or compatibility rule, it requires a
new profile or compatibility version and the full cryptographic release gates.

Removal of a vulnerable provider or target must fail explicitly. It must not
activate an unreviewed fallback to preserve availability.

## Current Crypto V2 provider set

The lockfile resolves this selected and license-reviewed Profile A set:

| Capability | Direct dependency | Relevant resolved dependencies | License |
|---|---|---|---|
| CSPRNG, SHA-256/512, HMAC-SHA256, HKDF-SHA256 | `ring 0.17.14` | `getrandom` through `ring` | Apache-2.0 AND ISC |
| X25519 | `x25519-dalek 2.0.1` | `curve25519-dalek 4.1.3`, `subtle 2.6.1`, `zeroize 1.8.2` | BSD-3-Clause |
| Ed25519 | `ed25519-dalek 2.1.1` | `curve25519-dalek 4.1.3`, `ed25519 2.2.3`, `signature 2.2.0`, `sha2 0.10.9`, `zeroize 1.8.2` | BSD-3-Clause |
| ChaCha20-Poly1305 | `chacha20poly1305 0.10.1`, `poly1305 0.8.0` | `aead 0.5.2`, `chacha20 0.9.1`, `zeroize 1.8.2` | Apache-2.0 OR MIT |
| Constant-time comparison | `subtle 2.6.1` | none | BSD-3-Clause |
| ML-KEM-768 (portable implementation on every target) | `libcrux-ml-kem 0.0.10` | `libcrux-sha3 0.0.10`, `libcrux-intrinsics 0.0.8`, `libcrux-platform 0.0.3`, `libcrux-secrets 0.0.6`, `libcrux-traits 0.0.8`, `hax-lib 0.3.7` (and the `hax-lib-macros 0.3.7` proc macro), `rand_core 0.10.1` through `rand 0.10.3` (trait bounds only) | Apache-2.0 (rand: MIT OR Apache-2.0) |
| Argon2id v1.3 | `argon2 0.5.3` | `blake2 0.10.6`, `base64ct 1.8.3`, `cpufeatures 0.2.17` on x86, `zeroize 1.8.2` | MIT OR Apache-2.0 |
| Blind RSA BR1: blind, finalize, EMSA-PSS | `crypto-bigint 0.7.5` | `ctutils 0.4.2`, `cmov 0.5.4`, `cpubits 0.1.1`, `num-traits 0.2.19`, `zeroize 1.8.2`; SHA-384 from `ring 0.17.14` | Apache-2.0 OR MIT |
| Blind RSA BR1: verify, and the check in finalize | `ring 0.17.14` (`RSA_PSS_2048_8192_SHA384`) | none | Apache-2.0 AND ISC |
| Blind RSA BR1: sign, generate, PKCS#8 import (Linux and macOS only) | `aws-lc-sys 0.45.0` (AWS-LC 5.7.0) | build only: `cc`, `cmake 0.1.58`, `dunce 1.0.5`, `fs_extra 1.3.0`, `pkg-config` | ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0) |

The new algorithm crates are exact-pinned with default features disabled; only
static-secret and zeroization features required by the public profile are
enabled. ML-KEM enables only the ML-KEM-768 parameter set (no `rand`, `std`,
SIMD or Kyber features); Mesh supplies the production CSPRNG bytes, stores the
64-byte FIPS 203 seed in an actor-owned zeroizing resource, and wipes the
expanded private key and shared secret itself (libcrux wipes nothing). The direct `poly1305` entry enables zeroization for
transitive MAC state. Production selects one static provider. The deterministic
provider is compiled only under `cfg(test)`, and there is no runtime algorithm
fallback.
Argon2 is exact-pinned with default features disabled and only its zeroization
feature enabled. Mesh supplies the bounded allocation, wraps the full working
memory plus temporary password and output copies in zeroizing storage, and
fixes the algorithm to Argon2id v1.3. The pure-Rust implementation is shared by
all supported targets; there is no target-specific or runtime-selected KDF.
The public API is binary-first, private keys remain actor-owned resources, and
official vectors plus negative and boundary tests cover every primitive. The
ML-KEM key-generation check through compiled Mesh is pinned to NIST ACVP
FIPS 203 `tcId 26`, and the runtime checks every ACVP ML-KEM-768 key
generation, encapsulation and decapsulation case plus an OpenSSL differential
(see the ML-KEM change review below); the
Argon2id check is pinned to the reference implementation's v1.3 raw vector for
`password` and `somesalt`.

Blind RSA follows the same rules, detailed in the change review below:
`crypto-bigint` is exact-pinned with default features disabled and only
`zeroize` enabled, `aws-lc-sys` is exact-pinned with default features disabled
(its prefixed universal bindings), and `ring` is now exact-pinned because Mesh
relies directly on its RSA-PSS verification.

`scripts/verify-crypto-mobile.sh` builds the complete runtime library for
`aarch64-apple-ios` and `aarch64-linux-android`, proves AWS-LC is absent
from both dependency graphs and archives, and that ML-KEM comes from
`libcrux-ml-kem 0.0.10` with the `ml-kem` crate absent. Tagged releases now require a clean RustSec audit, a
CycloneDX SBOM, and reproducible isolated `meshc` builds from the release
revision on the workflow host. Android and reproducibility of every advertised
target archive remain Milestone 10 release gates before production activation.

## Change review: blind RSA (profile BR1)

Reviewed 2026-09-29 for `Crypto.BlindRsa`. Everything below holds for the
lockfile of that change; an update to any listed version repeats this review.

**Capability and profile.** Privacy Pass token type `0x0002` (RFC 9578) needs
RSABSSA-SHA384-PSS-Deterministic (RFC 9474 §5): EMSA-PSS with SHA-384,
MGF1-SHA-384 and a 48-byte salt, identity message preparation, 2,048-bit keys,
e = 65,537, public keys as the RFC 9578 RSASSA-PSS SPKI. The public Mesh API
accepts nothing else; other sizes, exponents and the randomized variants fail
with a typed error and never fall back.

**Why the accepted set was not enough.** `ring` verifies RSA-PSS but has no
raw RSA private-key operation, no blinding and no big-integer arithmetic.
No accepted crate offers constant-time modular exponentiation and inversion
over a runtime modulus. The pure-Rust `rsa` crate was rejected: it carries an
open RustSec timing advisory (Marvin), and the release audit fails on any
finding.

**Exact versions.** Direct: `crypto-bigint =0.7.5` (`default-features = false`,
`features = ["zeroize"]`), `aws-lc-sys =0.45.0` (`default-features = false`),
`ring =0.17.14`. Transitive: `ctutils 0.4.2`, `cmov 0.5.4`, `cpubits 0.1.1`,
`num-traits 0.2.19`, `zeroize 1.8.2`; build-only for `aws-lc-sys`: `cc`,
`cmake 0.1.58`, `dunce 1.0.5`, `fs_extra 1.3.0`, `pkg-config`. `aws-lc-sys
0.45.0` builds AWS-LC 5.7.0 from source with its `cc` builder on Linux and
macOS (no CMake, NASM or bindgen needed there) and exports every symbol with
the `aws_lc_0_45_0_` prefix, so it cannot collide with a system OpenSSL.
`crypto-bigint` 0.7.0 to 0.7.4 are yanked; 0.7.5 (2026-06-22) fixes the
truncated Karatsuba carry that caused the last yank.

**Targets.** `crypto-bigint` and `ring` build everywhere, so blinding,
finalizing, verifying and SPKI parsing work on every target. `aws-lc-sys` is a
`cfg(any(target_os = "linux", target_os = "macos"))` dependency: iOS, Android
and Windows never compile it, and their key generation, import, public-key
derivation, signing and issuer-key sealing return
`CryptoError.UnsupportedTarget` (import still consumes its input).
`scripts/verify-crypto-mobile.sh` proves the absence for iOS and Android;
`cargo tree --target x86_64-pc-windows-msvc` shows it for Windows.

**Licenses and findings.** Licenses are listed in the provider table; AWS-LC's
combination is permissive (ISC, Apache-2.0, MIT, BSD-3-Clause, MIT-0) and
compatible with Mesh. `cargo audit --deny warnings` on the workspace lockfile
reports no vulnerability and no warning with these versions (advisory
database of 2026-09-29, 1,277 advisories).

**Secret memory, allocation and timing.**

- The blinding factor `r`, its inverse and the encoded message live in
  `crypto-bigint` values on the stack and zeroizing buffers; each is zeroized
  before return. The inverse is returned only as a `BlindRsaBlindingState`
  resource (kind 10).
- `r^e mod n` and every product use `FixedMontyForm` Montgomery arithmetic,
  constant-time in the secret operand; the exponent 65,537 is public, so
  `pow_vartime` leaks only it.
- `r^-1 mod n` uses `Uint::invert_odd_mod`, which calls
  `safegcd::invert_odd_mod::<LIMBS, false>` (`src/uint/invert_mod.rs:156-157`
  in 0.7.5). With `VARTIME = false` the Bernstein-Yang loop runs a fixed
  `iterations(Uint::<LIMBS>::BITS)` = (45,907 x bits + 30,179) / 19,929
  batches (`src/modular/safegcd.rs:117`, `:350`), each batch is 62
  `jump_step`s made of `Choice` selects with no data-dependent branch or index
  (`:185-210`), and the result is normalized with a select (`:500-503`). The
  only early exit (`:122`) is compiled out when `VARTIME` is false. The
  inversion is constant-time in the value inverted in this version.
- The check that `r` is below `n` is a constant-time comparison; a rejected
  `r` is discarded, so only the fact of a redraw is observable.
- Private keys are canonical PKCS#8 DER in a `BlindRsaSecretKey` resource
  (kind 9, 1,190 to 1,220 bytes). Each operation parses a zeroizing copy into
  AWS-LC; AWS-LC frees every `BIGNUM` with `OPENSSL_free`, which cleanses the
  allocation, and the marshalled PKCS#8 buffer is copied into a zeroizing box
  before `OPENSSL_free`.
- Signing is `RSA_sign_raw` with `RSA_NO_PADDING`; the runtime refuses a key
  with `RSA_FLAG_NO_BLINDING`, so AWS-LC's RSA blinding, constant-time CRT
  exponentiation and fault check always run. The runtime then checks
  RSAVP1(s) = m before releasing the signature (RFC 9474 §4.3).
- Verification is ring's `RSA_PSS_2048_8192_SHA384`, variable time over
  public values only.
- `scripts/verify-crypto-timing.sh` records a release-mode fixed-versus-random
  timing distribution for `Crypto.blind_rsa_sign`.

**Vectors, wire and downgrade.** New known answers: RFC 9578 Appendix A.2 (all
five type-2 tokens, through the public Mesh API and through the helpers and
AWS-LC) and RFC 9474 Appendix A.3 (4,096-bit, through the size-generic
helpers and AWS-LC). The OpenSSL CLI differential checks Mesh SPKIs and
signatures with `openssl dgst -sha384` under RSASSA-PSS with a 48-byte salt,
and raw signing against OpenSSL's raw private-key operation. The SPKI is
parsed against one byte template, so there is no negotiable parameter to
downgrade. New wire-visible identifiers: resource kinds 9 and 10, storage
purpose 18, and `CryptoError.UnsupportedTarget` appended after the existing
variants (existing tags unchanged).

## Change review: ML-KEM provider (libcrux-ml-kem)

Reviewed 2026-09-29 (Witness network plan C6, decision D17). Everything below
holds for the lockfile of that change; an update to any listed version repeats
this review.

**Capability and profile.** ML-KEM-768 (FIPS 203) for the hybrid messenger
suite: 64-byte seeds laid out as d then z (`ML-KEM.KeyGen_internal`), the
FIPS 203 section 7.2 encapsulation-key check, deterministic
`ML-KEM.Encaps_internal` from 32 bytes of Mesh CSPRNG output, and implicit
rejection on decapsulation. The public API, encodings (1,184-byte public
keys, 1,088-byte ciphertexts, 32-byte shared secrets), the `MlKemPrivateKey`
resource (kind 8, the 64-byte seed) and its storage-sealing format (purpose
15) are unchanged, so sealed keys and wire data from earlier builds stay
valid.

**Why the accepted set was not enough.** `ml-kem 0.3.2` (RustCrypto) had no
audit or proof. `libcrux-ml-kem` is Cryspen's ML-KEM, verified with hax and
F* and used by Signal. The swap is behaviour-preserving: the same vectors pass
before and after.

**Exact versions.** Direct: `libcrux-ml-kem =0.0.10` with
`default-features = false, features = ["mlkem768"]`. Transitive and compiled:
`libcrux-sha3 0.0.10`, `libcrux-intrinsics 0.0.8`, `libcrux-platform 0.0.3`
(runtime CPU detection, with `libc 0.2.180`), `libcrux-secrets 0.0.6`,
`libcrux-traits 0.0.8`, `hax-lib 0.3.7`, the proc macro `hax-lib-macros 0.3.7`
(build time only, with `proc-macro2`, `quote`, `syn`), and `rand 0.10.3` with
`rand_core 0.10.1`, which `libcrux-traits` names in trait bounds (no generator
is compiled or called). The lockfile also lists dependencies behind
`cfg(hax)` (`core-models`, `num-bigint`, `hax-lib-macros-types`, `uuid`,
`proc-macro-error2`) and `cfg(valgrind_ct_test)` (`crabgrind`, `bindgen` and
its tree); those cfgs are set only by Cryspen's proof and Valgrind tooling,
so no Mesh build compiles them. Removed: `ml-kem 0.3.2`, `module-lattice`,
`hybrid-array`, `kem` and `sha3 0.11.0`.

**Targets, and which implementation runs.** libcrux ships a portable backend
and NEON (aarch64) and AVX2 (x86_64) backends. Its build scripts compile the
NEON backend into every aarch64 build (including `aarch64-apple-ios`,
`aarch64-linux-android` and Apple Silicon macOS) and the AVX2 backend into
every x86_64 build (Linux, Intel macOS, Windows), whatever features are
selected, and its top-level `mlkem768::{generate_key_pair, encapsulate,
decapsulate}` pick one at run time from CPU detection. Mesh never calls those.
It calls `mlkem768::portable::{generate_key_pair, validate_public_key,
encapsulate, decapsulate}` directly, which instantiate the portable vector
type and the portable Keccak (`libcrux_sha3::portable`), so every target runs
the same portable code and the SIMD backends are dead code. Nothing else in
the runtime reaches libcrux.

**Verification status.** Per libcrux's `libcrux-ml-kem/proofs/verification_status.md`
at the published source (`c5fb80f3`, table generated 2026-03-10): the
portable backend's arithmetic, NTT, serialization, compression and sampling
(52 functions) are proved panic-free and correct; the AVX2 backend is mostly
proved; the NEON backend is unverified apart from sampling (the reason Mesh
does not use it). Of the shared generic code, `ind_cca`, `polynomial`,
`hash_functions`, `constant_time_ops` and the ML-KEM instantiations are proved
(`ind_cca` 26 of 27, `mlkem` 128 of 134 correct), while `ind_cpa`, `matrix`,
`sampling`, `ntt`/`invert_ntt` and `serialize` are partly or not yet proved.
`libcrux-sha3` is marked pre-verification. So the provider is largely,
not completely, formally verified; the vectors and differential below cover
the unproved parts behaviourally, and the outside review (plan C10) is still
to come. libcrux's `check-secret-independence` feature (compile-time
secret-independence typing) does not compile in 0.0.10, so it is not enabled.

**Licenses and findings.** Apache-2.0 throughout (`rand`, `rand_core`: MIT OR
Apache-2.0), compatible with Mesh. `cargo audit --deny warnings` on the
workspace lockfile (advisory database of 2026-09-29, 1,277 advisories) reports
one finding: RUSTSEC-2026-0173, `proc-macro-error2 2.0.1` unmaintained. It is
a `cfg(hax)` dependency of `hax-lib-macros`, compiled only by the hax proof
toolchain and never by Mesh; it is listed as a reviewed exception in
`.cargo/audit.toml`, after which the audit is clean.

**Secret memory, allocation and timing.** libcrux allocates nothing and wipes
nothing. Mesh copies the seed into a zeroizing array, wraps the expanded
2,400-byte private key in `ExpandedMlKemKey`, which wipes it on drop, and
copies each 32-byte shared secret into a zeroizing box before wiping the
array libcrux returned. The by-value copies libcrux's API makes and its stack
temporaries (the secret vector in NTT form, re-encryption state) are not
wiped; `ml-kem 0.3.2` wiped only its key structs, not those either. libcrux
follows constant-time patterns in the portable code and uses its
`libcrux-secrets` integer types for ring arithmetic; there is no timing
guarantee beyond that.

**Vectors, wire and downgrade.** New: `tests/vectors/mlkem/mlkem768-acvp-fips203.json`,
every ML-KEM-768 case of NIST ACVP-Server `65370b86` (25 key generations
checked against both ek and the expanded dk, 25 encapsulations, 10
decapsulations including modified ciphertexts on the implicit-rejection
path), and `tests/vectors/mlkem/mlkem768-openssl.json`, eight random seeds run
through OpenSSL 3.6.3 (public key, deterministic encapsulation with `ikme`,
decapsulation of the ciphertext and of a copy with one flipped bit), both
checked by `crypto::mlkem_tests` in `mesh-rt`; `bash
scripts/generate-mlkem-vectors.sh` regenerates them. The existing ACVP
`tcId 26` vector through compiled Mesh, the negative length and unreduced-key
tests, and Morse's OpenSSL interop vector pass unchanged. No encoding, domain
label, wire identifier or negotiation changes, so there is nothing to
downgrade.
