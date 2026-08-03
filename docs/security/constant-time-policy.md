# Constant-Time Policy

Status: required security contract; current Mesh code has no compiler-enforced
constant-time guarantee.

This policy applies whenever secret data can influence computation. A
contributor should use it to choose between a runtime-backed primitive and a
future direct Mesh implementation, and to determine the evidence required for
either path.

## Production policy

Reviewed, runtime-backed primitives are the production path. Mesh source must
not implement cryptographic algorithms directly until the constant-time
compiler restrictions and release verification in this policy are complete.

Runtime-backed code must:

- Use the profile-selected, pinned provider through the public Mesh API.
- Use binary inputs and outputs, with secret inputs held by secret/resource
  types.
- Use a reviewed constant-time dependency for equality and selection instead
  of a custom accumulation loop.
- Validate lengths and bounds before calling the provider and return typed
  errors.
- Fail clearly on unsupported targets or algorithms; it must not fall back to
  another algorithm.

Constant-time behavior concerns control flow and memory access derived from
secret values. Public framing, algorithm identifiers, and validated public
lengths may be handled normally, but they must not be confused with secret
material.

## Future `@constant_time` contract

Inside a future `@constant_time` function, secret-derived values must not:

- Control branches, switch tables, early returns, or loop counts.
- Control memory indexes, pointer arithmetic, allocation sizes, or bounds
  checks.
- Enter formatting, diagnostics, or panic messages.
- Reach an intrinsic or function that is not approved for constant-time use.

Only fixed-width integer operations are allowed, and wrapping arithmetic must
be explicit. The compiler must enforce these rules in restricted MIR after
type checking and reject unsupported operations rather than relying on source
inspection.

## Verification

Every runtime-backed primitive requires known-answer vectors, negative tests,
fuzzing, bounds tests, cross-platform builds, and provider/dependency review.
Comparison tests must include unequal lengths at byte-width boundaries,
including a length difference of 256.

A direct Mesh cryptographic implementation additionally requires all of the
following for every supported release configuration:

1. Reference-vector comparison.
2. Differential testing against an independent implementation.
3. Generated machine-code inspection.
4. Timing-distribution testing where meaningful.
5. Verification at every supported optimization level.
6. Verification on every supported architecture.
7. External specialist review.

Timing tests are supporting evidence, not a substitute for compiler
restrictions, code inspection, or review.

## Current limitations

- Mesh has no `@constant_time` annotation, secret-taint analysis, restricted
  MIR verifier, or constant-time-approved call graph.
- `SecretBytes`, affine resource ownership, compiler-inserted destruction, and
  bounded generational runtime storage are implemented; release approval is
  still pending the remaining cryptographic evidence.
- The unsafe string comparison API has been removed. `Bytes.secure_equals`
  uses the runtime's constant-time dependency and has a 256-byte length-boundary
  regression test. `scripts/verify-crypto-timing.sh` records a release-mode
  timing-distribution check for equal-length first- and last-byte mismatches.
- Binary-first hash, HMAC, HKDF, AEAD, signature, key-agreement, and KEM APIs
  use the selected runtime provider and secret/resource inputs where required.

## Migration order

Direct Mesh implementations, if pursued after the compiler enforcement lands,
should progress from HKDF composition, HMAC composition, and constant-time
select/compare to selected symmetric primitives and AEAD. Curve and lattice
arithmetic comes only much later. This research track does not block the
messenger MVP.

No primitive may be called constant-time in release material unless it passes
the applicable [cryptographic release gates](cryptographic-release-gates.md).
