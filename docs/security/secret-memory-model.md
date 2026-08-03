# Secret Memory Model

Status: required security contract; secure runtime subset implemented, release
approval pending.

This policy defines how Mesh must represent secret key material. It applies to
private keys, shared secrets, ratchet keys, message keys, recovery material,
and storage-wrapping keys. A contributor should use it to decide whether a new
type or API is safe to receive, retain, or destroy secret material.

## Current baseline

`Bytes` is GC-managed binary data. It can be copied, formatted through
surrounding values, serialized, sent between actors, and retained until GC
collection. It does not provide timely erasure, move-only ownership, automatic
actor cleanup, or use-after-destroy detection.

Mesh now implements `SecretBytes`, generalized affine resources, compiler-
inserted destruction, and a bounded generational resource table. Unsupported
resource closure capture is rejected until closure environments carry affine
metadata. Until the remaining release evidence exists:

- Ordinary `Bytes` and `String` are not approved containers for private keys or
  ratchet material.
- Legacy string-first cryptographic APIs do not satisfy this policy.
- Features that require retained secret state must not claim compliance with
  the secure-memory model.

## Language contract

`SecretBytes` is the initial compiler-known secret type. It must be:

- Opaque and move-only.
- Borrowed only for a direct call in the initial ownership model.
- Explicitly consumable and destructible.
- Automatically destroyed on every scope exit.
- Non-printable and non-debuggable.
- Non-JSON, non-row, non-schema, non-hashable, and non-serializable.
- Ineligible for ordinary equality.
- Ineligible for ordinary unrestricted collections.
- Ineligible as an actor message or cross-node value.

The same restrictions apply transitively to any value containing a secret or
resource. Assignment moves such values by default, closure capture moves
ownership, and use after a move is a compile error. All normal, error,
early-return, loop, match, closure, actor-termination, and failure exits must
destroy each live resource exactly once.

Cryptographic APIs may borrow a secret when they only read it and consume one
when ownership ends. They must never expose a secret handle as `Int` or turn
secret bytes into ordinary Mesh data as an implementation shortcut.

## Runtime contract

Secrets must live in a bounded generational resource table. A Mesh value is an
opaque `{slot, generation, kind}` handle. Each runtime entry records its owner
actor, resource kind, zeroizing allocation, live state, and byte count.

The runtime must:

- Check the slot, generation, kind, owner, and live state on every operation.
- Invalidate the generation when a resource is destroyed.
- Reject stale handles and use after destroy with a typed error.
- Zeroize the secret allocation before releasing it.
- Destroy all resources owned by an exiting actor.
- Bound both secret count and total secret bytes per actor.
- Reject cross-actor and cross-node transfer.
- Make explicit destroy and internal cleanup idempotent.
- Redact secret values from every runtime diagnostic path.

Compiler-inserted scope drops and runtime actor cleanup are independent safety
nets. Neither may be omitted because the other exists.

## Persistent secrets

Persistent secret state must be sealed by a `StorageKey`; it must never be
stored as plaintext `Bytes`. A sealed blob must contain a version, algorithm
identifier, unique nonce, ciphertext, authentication tag, and context binding.

The context must bind the account, device, session, secret purpose, and
snapshot version. A mismatched context must fail authentication without
returning plaintext. Mobile hosts store or wrap the `StorageKey` with Keychain
or Keystore and follow the versioned storage-wrapping callback contract.
`StorageKey.ephemeral()` is process-local and exists for short-lived tools and
tests; blobs sealed with it cannot be restored after that process exits.
Storage context purpose `15` is reserved for ML-KEM prekey seeds and accepts
only `MlKemPrivateKey`; resource kind `8` is used because value-kind `7` already
identifies ordinary sealed bytes.

## Required evidence

The model is not complete until tests prove:

- Move, borrow, consume, use-after-move, and containing-value restrictions.
- Rejection by formatting, derivation, serialization, collections, actor send,
  and cross-node transfer.
- Stale-handle, wrong-owner, wrong-kind, count-quota, and byte-quota failures.
- Explicit destruction, every compiler-inserted drop path, and actor-exit
  cleanup.
- Storage seal/unseal round trips and context-mismatch rejection.
- Sentinel secret material is absent from errors, panic output, logs,
  telemetry, and crash reports.

The remaining cryptographic evidence is defined by the
[cryptographic release gates](cryptographic-release-gates.md).
