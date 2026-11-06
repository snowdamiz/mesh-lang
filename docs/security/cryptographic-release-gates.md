# Cryptographic Release Gates

Status: mandatory gates for any Mesh release that adds or changes a
cryptographic primitive, provider, secret type, or supported cryptographic
profile.

These gates distinguish implementation progress from release approval. Passing
unit tests alone is not sufficient, and development-profile evidence must not
be presented as production security review.

## Gate 1: Profile and compatibility

- The consuming protocol names a versioned cryptographic profile.
- Algorithms, parameters, domain-separation labels, wire identifiers, and
  downgrade behavior are explicit.
- The selected suite is authenticated and included in associated data where
  applicable.
- Unsupported higher suites fail explicitly; there is no silent algorithm
  fallback.
- Compatibility and migration behavior have tests.

## Gate 2: Public Mesh contract

- A binary-first public Mesh API exists.
- Secret keys and retained key material use secret/resource types, never
  ordinary `Bytes` or `String`.
- Inputs and allocations are bounded and errors are typed.
- Type checking, code generation, and runtime ABI agree.
- The same public API is used by tests, examples, and production code.
- The messenger consumes the public API; no private application-level Rust
  protocol or cryptographic implementation exists.

## Gate 3: Provider and dependencies

- The default provider and dependency versions are pinned.
- Deterministic providers are confined to compile-time test/fuzz builds,
  never selectable through a production runtime or public Mesh API.
- Unsupported targets fail clearly.
- Dependency provenance, security findings, versions, and licenses are
  reviewed under the [dependency policy](dependency-policy.md).
- No dependency changes the selected algorithm or compatibility behavior
  implicitly.

## Gate 4: Verification evidence

Evidence must be generated from the release commit and include:

- Published known-answer vectors and an automated vector runner.
- Negative tests for malformed inputs, wrong lengths, wrong keys, tampering,
  authentication failure, and allocation limits.
- Fuzz harnesses for every public parser and cryptographic boundary.
- Regression coverage for known defects, including secure-comparison length
  boundaries.
- Secret-leak sentinel tests across errors, panic paths, logs, telemetry, and
  crash reports.
- Cross-platform builds for every advertised target.
- Machine-readable test reports and coverage results.
- A dependency audit, software bill of materials, and reproducible-build
  check.

Plaintext must never be returned after an authentication failure. Tests must
exercise the actual public API and provider used in production.

## Gate 5: Constant-time assurance

Runtime-backed primitives must use the approved provider and constant-time
dependency as defined by the [constant-time policy](constant-time-policy.md).

A cryptographic algorithm implemented directly in Mesh also requires
differential tests, generated-machine-code inspection, timing-distribution
tests, and all supported optimization levels and architectures. External
specialist review is valuable additional scrutiny, not a release prerequisite.

## Gate 6: Integration proof

- The messenger pins the exact Mesh commit or release in CI.
- Interoperability tests cross the Mesh API and application boundary.
- Protocol codecs, snapshots, and stored secret blobs have explicit versions.
- Mobile builds execute the same vectors on every supported platform before
  that platform is advertised.
- Production-target profiles require internally verified evidence for the exact
  candidate and applicable platforms. Independent review is reported accurately
  and is not an activation requirement.

## Stop-ship conditions

A release is blocked if any applicable gate lacks current evidence, or if it
contains:

- A known correctness defect in authentication, comparison, key derivation,
  nonce handling, or secret destruction.
- Secret material in ordinary strings, logs, diagnostics, telemetry, crash
  reports, or serialized actor messages.
- An undocumented algorithm, fallback, domain label, or profile change.
- An unresolved dependency finding without an explicit security disposition.
- A target that silently substitutes or omits the selected primitive.
- Stale evidence produced from a different source revision.

## Current baseline

The development runtime implements the classical Crypto V2 API, an ML-KEM-768
KEM primitive, and an Argon2id v1.3 password KDF, plus affine secret resources,
a static production provider, a test/fuzz-only deterministic provider, known-answer
and negative tests, a published NIST ACVP ML-KEM-768 key-generation vector
executed through the public Mesh API, and an iOS compilation proof. The hybrid
messenger suite is callable; there is no software-enforced external-review
disable. Release readiness requires protocol negotiation, downgrade and target
performance evidence at the exact candidate. No independent audit is claimed:

- Native fuzz targets now exercise byte operations, including bounded
  Argon2id calls through the production provider,
  runtime protocol/routing/WebSocket decoders, and the lexer/parser. Coverage-
  guided targets for generated Mesh messenger boundaries and the complete
  secret-leak sentinel suite remain open.
- The full advertised mobile/host target matrix remains a Milestone 10 gate.
- The release workflow now blocks tagged releases on a clean RustSec audit, a
  CycloneDX SBOM, and two reproducible isolated `meshc` builds for the workflow
  host. It also records a release-mode timing-distribution check for
  `Bytes.secure_equals` and BR1 raw signing, runs the published ML-KEM and
  RFC 9578 Type 2 vectors through compiled Mesh, compares BR1 and ML-KEM-768
  with OpenSSL (ML-KEM requires OpenSSL >=3.5),
  and packages the vectors and test log. Published external vectors for
  the remaining Crypto V2 primitives and reproducibility of every advertised
  target archive remain open.

Current primitives are implementation baseline, not approval evidence. A
release record must name the profile, Mesh revision, provider and dependency
versions, tested targets and optimization levels, vector set, test reports,
audit/SBOM/reproducibility results, known limitations, and required review.

Run `bash scripts/generate-crypto-release-evidence.sh OUTPUT_DIRECTORY` from a
clean release revision to produce the audit JSON, CycloneDX 1.5 SBOM,
secure-equality and blind-RSA-signing timing records, reproducibility
comparison, release record, published ML-KEM/RFC 9578/RFC 9474 vectors,
public-Mesh/OpenSSL vector test log, logs, and checksums. The
output record explicitly lists the remaining vector, fuzzing, target-matrix,
secret-leak evidence that this command does not provide. Independent review
is recorded separately and is not an approval credential.

## BR1 evidence contract

BR1 is RSABSSA-SHA384-PSS-Deterministic: RSA-2048/e65537, SHA-384,
MGF1-SHA-384, a 48-byte salt and canonical RFC 9578 RSASSA-PSS SPKI. Private
operations are limited to Linux/macOS and return `UnsupportedTarget`
elsewhere; the client operations and sealed blinding-state recovery remain
available on every target. A local development pass is not evidence from a
clean release revision.

| Surface | Runner and expected evidence |
|---------|------------------------------|
| Published public contract | `cargo test --locked -p meshc --test e2e_crypto_v2 blind_rsa_`: all five RFC 9578 A.2 signatures and exact issuer responses through compiled Mesh, fresh blind/sign/finalize, bounds and malformed inputs, sealed key/state recovery, and OpenSSL PSS/raw-RSA comparison |
| Larger reference vectors | Runtime `crypto::blind_rsa::tests` consumes RFC 9474 A.1–A.4 with the compile-time test/fuzz provider; accepting these 4096-bit helper vectors must not broaden the BR1 public profile |
| Ownership and ABI | Type-checker, MIR, intrinsic, JIT and runtime ABI tests must cover both affine resources and every public operation; the public e2e cannot replace these checks |
| Runtime misuse | Stale/wrong-owner/wrong-kind/destroyed handles, consume-on-error, allocation limits and secret-leak sentinels are mandatory runtime evidence |
| Timing | `verify-crypto-timing.sh` requires one release-mode record for `Bytes.secure_equals` and one for `Crypto.blind_rsa_sign`; schema 3 contains both schema 2 boundary records. Identical-input controls distinguish noisy measurements; `inconclusive` must remain visible and requires a quiet-host rerun before assurance claims |
| Phone compilation | `verify-crypto-mobile.sh` defaults to both `aarch64-apple-ios` and `aarch64-linux-android`, checks client symbols and rejects AWS-LC in the target dependency graph/static library; JSON explicitly says `compile_only: true`, `vectors_executed: false` |
| Windows client | The compatibility matrix runs `blind_rsa_client_public_api_all_targets`, including published verification, client state storage, finalization rejection and the server `UnsupportedTarget` gate |
| Executed device proof | Physical iOS and Android public-API vectors and application issuance/redeem remain separate evidence; successful cross-compilation cannot close this gate |

The fixture directory records source URLs, source-text hashes, derived-field
rules and the published-test-key warning. The runtime/provider change review
is [blind-rsa-provider-review.md](blind-rsa-provider-review.md). A release must
also retain its clean audit, license review, SBOM, executed fuzz/leak tests,
reproducible artifacts and exact target matrix. No outside review, remote CI
run, device execution or tagged release is implied by adding these runners.
