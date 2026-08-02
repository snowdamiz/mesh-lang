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

The new algorithm crates are exact-pinned with default features disabled; only
static-secret and zeroization features required by the public profile are
enabled. The direct `poly1305` entry enables zeroization for transitive MAC
state. Production selects one static provider. The deterministic provider is
compiled only under `cfg(test)`, and there is no runtime algorithm fallback.
The public API is binary-first, private keys remain actor-owned resources, and
official vectors plus negative and boundary tests cover every primitive.

`scripts/verify-crypto-mobile.sh` checks the complete runtime library for
`aarch64-apple-ios`. Android and artifact reproducibility remain Milestone 10
release gates. A current advisory audit, SBOM, and reproducible-build record are
still required from the final release revision before production activation.
