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
- Any deterministic test provider is compile-time test-only.
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
tests, all supported optimization levels and architectures, and external
specialist review.

## Gate 6: Integration proof

- The messenger pins the exact Mesh commit or release in CI.
- Interoperability tests cross the Mesh API and application boundary.
- Protocol codecs, snapshots, and stored secret blobs have explicit versions.
- Mobile builds execute the same vectors on every supported platform before
  that platform is advertised.
- Production-target profiles receive independent security review before a
  production security claim is made.

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

The development runtime implements the classical Crypto V2 API plus an
ML-KEM-768 KEM primitive, affine secret resources, a static production provider,
a test-only deterministic provider, known-answer and negative tests, and an iOS
compilation proof. The hybrid messenger suite remains disabled until its
protocol negotiation, downgrade, target-performance, and external-review gates
pass. This is not yet a release-approved profile:

- The cryptographic fuzz and complete secret-leak sentinel suites remain open.
- The full advertised mobile/host target matrix remains a Milestone 10 gate.
- The release workflow now blocks tagged releases on a clean RustSec audit, a
  CycloneDX SBOM, and two reproducible isolated `meshc` builds for the workflow
  host. It also records a release-mode timing-distribution check for
  `Bytes.secure_equals`. Reproducibility of every advertised target archive
  remains open.

Current primitives are implementation baseline, not approval evidence. A
release record must name the profile, Mesh revision, provider and dependency
versions, tested targets and optimization levels, vector set, test reports,
audit/SBOM/reproducibility results, known limitations, and required review.

Run `bash scripts/generate-crypto-release-evidence.sh OUTPUT_DIRECTORY` from a
clean release revision to produce the audit JSON, CycloneDX 1.5 SBOM,
secure-equality timing record, reproducibility comparison, release record,
logs, and checksums. The output record explicitly lists the fuzzing,
target-matrix, secret-leak, and independent review evidence that this command
does not provide.
