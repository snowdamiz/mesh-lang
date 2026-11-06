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
- A deterministic provider is confined to compile-time test/fuzz builds and
  cannot be selected at runtime in production.
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

### Public Edwards25519 membership and Solana address derivation

The additive `Crypto.ed25519_is_on_curve(Bytes)` API reuses the selected
`ed25519-dalek 2.1.1` provider and `curve25519-dalek 4.1.3`. It adds no runtime
dependency or secret operation. Inspection of `VerifyingKey::from_bytes` in
that exact version confirms that it invokes compressed-point decompression,
without the weak-key rejection used by `verify_strict`. The API deliberately
accepts weak and noncanonical field encodings that decompress. A wrong-length
input is `InvalidPublicKey`; a 32-byte off-curve input is `Ok(false)`.
No existing signature-verification or wire-format behavior changes.

This predicate uses public data only and has no constant-time claim. Its output
is a GC-allocated Boolean result; it creates no key resources, parses no private
material and uses no random bytes. It follows the same provider path on host,
iOS and Android targets. `Solana.Address` performs the public hashing, framing
and bump search in Mesh; no application or Solana logic enters the runtime.

`meshc` has the test-only oracle dependency `solana-address =2.7.0` (Apache-2.0).
The oracle's relevant locked dependencies are `curve25519-dalek 4.1.3`
(BSD-3-Clause), `solana-sha256-hasher 3.1.0`, `solana-hash 4.7.0`,
`solana-program-error 3.0.1`, `solana-define-syscall 4.0.1/5.2.0` (Apache-2.0),
`sha2-const-stable 0.1.0` (MIT OR Apache-2.0), and
`five8/five8_const/five8_core 1.0.0` (MIT). These licenses are compatible with
the repository. This oracle is not linked into the release compiler/runtime.
Its curve provider is shared with Mesh, so the comparison is an SDK/wire
compatibility proof, not an independent implementation of curve arithmetic.

Tests include RFC 8032, weak/noncanonical points, wrong lengths, 512 SDK point
comparisons, 256 PDA derivations with empty and maximum-size seeds, rejection
of noncanonical bumps and input bounds, native compilation, the raw and
first-class API forms, intrinsic ABI, JIT symbol registration and the provider
fuzz surface. Release evidence still requires the current revision's dependency
audit, SBOM, reproducibility and advertised-target gates; local proof logs
do not replace those artifacts.

### Selected provider versions

The lockfile resolves this selected and license-reviewed Profile A set:

| Capability | Direct dependency | Relevant resolved dependencies | License |
|---|---|---|---|
| CSPRNG, SHA-256/512, HMAC-SHA256, HKDF-SHA256 | `ring 0.17.14` | `getrandom` through `ring` | Apache-2.0 AND ISC |
| X25519 | `x25519-dalek 2.0.1` | `curve25519-dalek 4.1.3`, `subtle 2.6.1`, `zeroize 1.8.2` | BSD-3-Clause |
| Ed25519 | `ed25519-dalek 2.1.1` | `curve25519-dalek 4.1.3`, `ed25519 2.2.3`, `signature 2.2.0`, `sha2 0.10.9`, `zeroize 1.8.2` | BSD-3-Clause |
| ChaCha20-Poly1305 | `chacha20poly1305 0.10.1`, `poly1305 0.8.0` | `aead 0.5.2`, `chacha20 0.9.1`, `zeroize 1.8.2` | Apache-2.0 OR MIT |
| Constant-time comparison | `subtle 2.6.1` | none | BSD-3-Clause |
| ML-KEM-768 | `libcrux-ml-kem 0.0.11` | `libcrux-intrinsics 0.0.9`, `libcrux-platform 0.0.4`, `libcrux-secrets 0.0.7`, `libcrux-sha3 0.0.11`, `libcrux-traits 0.0.9`; adapter uses `zeroize 1.8.2` | Apache-2.0; `zeroize` Apache-2.0 OR MIT |
| Argon2id v1.3 | `argon2 0.5.3` | `blake2 0.10.6`, `base64ct 1.8.3`, `cpufeatures 0.2.17` on x86, `zeroize 1.8.2` | MIT OR Apache-2.0 |
| BR1 client arithmetic | `crypto-bigint 0.6.1` | `subtle`, `zeroize` as locked | Apache-2.0 OR MIT |
| BR1 issuer RSA (Linux/macOS only) | `aws-lc-sys 0.46.0` | build-time `cmake 0.1.58`, `dunce 1.0.5`, `fs_extra 1.3.0`; bundled AWS-LC | ISC/Apache-2.0/MIT/BSD-3-Clause/MIT-0 components; full expression in the BR1 provider review |

The new algorithm crates are exact-pinned with default features disabled; only
static-secret and zeroization features required by the public profile are
enabled. ML-KEM enables only the `mlkem768` feature, using explicit portable
arithmetic and portable SHA3 entry points. Mesh supplies the production CSPRNG
bytes and stores the 64-byte FIPS 203 seed in an actor-owned zeroizing resource.
The mandatory `libcrux-traits` dependency includes `rand 0.10.3` with default
features disabled; Mesh does not call that RNG interface. The adapter explicitly
wipes its owned expanded private key because upstream's packed private-key
container has no zeroizing destructor. See the implementation and verification
limits in [mlkem-provider-review.md](mlkem-provider-review.md).
The direct `poly1305` entry enables zeroization for
transitive MAC state. Production selects one static provider. The deterministic
provider is compiled only under `cfg(any(test, feature = "fuzzing"))` for
vectors and bounded fuzz boundaries, with no production runtime selector or
algorithm fallback.
Argon2 is exact-pinned with default features disabled and only its zeroization
feature enabled. Mesh supplies the bounded allocation, wraps the full working
memory plus temporary password and output copies in zeroizing storage, and
fixes the algorithm to Argon2id v1.3. The pure-Rust implementation is shared by
all supported targets; there is no target-specific or runtime-selected KDF.
The public API is binary-first, private keys remain actor-owned resources, and
official vectors plus negative and boundary tests cover every primitive. The
ML-KEM key-generation check is pinned to NIST ACVP FIPS 203 `tcId 26`; the
Argon2id check is pinned to the reference implementation's v1.3 raw vector for
`password` and `somesalt`.

`scripts/verify-crypto-mobile.sh` checks release runtime compilation for both
`aarch64-apple-ios` and `aarch64-linux-android`, the BR1 client exports, and
exclusion of the private RSA provider from each target. This is compilation
and provider-exclusion evidence; on-device vectors are a separate gate. Tagged releases now require a clean RustSec audit, a
CycloneDX SBOM, and reproducible isolated `meshc` builds from the release
revision on the workflow host. Execution on each advertised target and
reproducibility of every target archive remain release gates before activation.

### BR1 blind RSA

BR1 additionally relies directly on `ring 0.17.14` for
`RSA_PSS_2048_8192_SHA384` verification, `crypto-bigint =0.6.1` with default
features disabled and zeroization enabled for client arithmetic, and
`aws-lc-sys =0.46.0` with default features disabled for issuer operations.
AWS-LC is a target-specific dependency only for `target_os = linux` or
`macos`; phones and Windows must neither build nor link it. Public SPKI
parsing does not activate an alternative provider. The exact constant-time,
license, transitive-version and advisory review is recorded in
[blind-rsa-provider-review.md](blind-rsa-provider-review.md).

The public conformance runner uses the OpenSSL CLI as an independent
implementation for PSS verification and raw RSA signing. It is test tooling,
not a runtime dependency or provider fallback. The RFC 9474 vectors using
other moduli/variants are confined to test helpers; the public API rejects
those profiles. No production runtime knob selects deterministic entropy.

Supply-chain approval still requires the release revision's clean audit,
complete SBOM, license evidence and reproducibility. A local dependency
resolution or passing vector test does not substitute for those artifacts.
