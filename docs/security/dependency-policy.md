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

## Current baseline

The runtime currently depends directly on `sha2`, `hmac`, `rand`, `ring`, and
`subtle` for several cryptographic and transport needs. This graph is not yet a
formal Crypto V2 provider set:

- There is no provider abstraction or deterministic test-only provider.
- `Bytes.secure_equals` uses the constant-time dependency; Crypto V2 still
  needs provider-wide review and evidence.
- The public crypto surface remains string-first.
- No dependency has yet been selected for the planned persistent X25519 key
  resource.
- Cryptographic dependency audit, SBOM, reproducibility, and release-record
  gates are not yet established.

The current graph is an inventory, not approval for the planned messenger
profile. New primitives remain blocked until they satisfy the
[cryptographic release gates](cryptographic-release-gates.md).
