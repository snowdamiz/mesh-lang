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

Server processes ingest hexadecimal private material with
`Env.get_secret_hex(name)`, which returns actor-owned `SecretBytes` without
creating a Mesh `String` or `Bytes` for the value. `Crypto.x25519_from_secret`,
`Crypto.signing_from_secret`, and `Crypto.mlkem_from_secret` consume that
resource directly into the corresponding private-key resource.

`Crypto.hmac_sha512_tag` borrows a `SecretBytes` key and publishes a 64-byte
protocol authentication tag over a bounded binary message. It never publishes
the key or converts the key to ordinary Mesh data. The tag is deliberately
ordinary `Bytes`, so this API must not be used as a secret-key derivation
operation. Existing secret-output HMAC/HKDF operations retain their behavior.

The development API also provides three operations for composing protocols
without exporting their retained secrets:

| Operation | Ownership and result |
|---|---|
| `Crypto.hkdf_sha256_secret_salt(input, salt, info, length)` | Borrows both `SecretBytes` inputs and returns a new `SecretBytes`. Argument order is input key material, secret salt, public info, output length. |
| `Crypto.hmac_sha256_tag(key, message)` | Borrows a `SecretBytes` key and publishes the full 32-byte authentication tag as `Bytes`. It is not a secret-output KDF. |
| `Secret.split_at(source, index)` | Consumes `SecretBytes` and returns two nonempty `SecretBytes` containing its exact prefix and suffix. A valid owned input is consumed on failure too; neither partial result is published. |

HKDF accepts output lengths from 1 through 8,160 bytes and public info up to
64 KiB. The tag API accepts messages up to 64 KiB. Splitting requires an index
strictly inside the input; it neither exposes its private length in an error
nor adds a generic secret-to-`Bytes` conversion. Allocation, resource quota,
generation and owner checks still apply. These operations have focused host
and compiled-Mesh coverage; their current combined native/release qualification
remains separate from that development evidence.

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

### Deliberate recovery-code setup disclosure

`RecoveryCode.generate()` is a narrow exception for newly created user-held
recovery material: it returns an actor-owned `SecretBytes` and the corresponding
canonical display `String`. It accepts no existing key, and no redisplay or
generic secret-to-ordinary-data conversion is added. `RecoveryCode.parse()`
imports only that bounded, checksummed representation into protected storage.
Normal resource ownership and fail-closed allocation still apply. Temporary
native buffers are zeroized; the deliberately returned String and UI copies
cannot promise prompt erasure. The application must show the code only during
setup, clear it afterwards and never log, persist or upload it. See
[the recovery-code contract](../reference/recovery-code.md).

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

### Protected runtime allocations

The resource table retains its byte material in runtime-owned anonymous memory
mappings. Prepared storage-key and owned-resource copies, consumed resources,
random-secret and concatenation outputs, and decoded/encoded `SecretMap` values
use the same protected allocator. A consumed resource keeps its protected
allocation until its final owner drops it; leaving the table does not unlock it.
An internal provider buffer imported into the table is copied only after the
destination is protected, and the old zeroizing buffer is then erased on drop.

No new Mesh API exposes this allocator. Resource generation, kind, actor
ownership and byte/count quotas still apply. An operating-system protection
failure returns `CryptoError.InternalFailure`; an unsupported platform returns
`CryptoError.UnsupportedTarget`; the allocator's mapped-memory limit returns
`CryptoError.ResourceLimitExceeded`. There is no ordinary-heap fallback. A
failed import publishes no handle and charges no actor usage. Failed replacement
leaves the original resource intact. Release initialization stops the process if
the required process dump policy cannot be established, before actors or
providers initialize.

Allocations use disjoint, 16-byte-aligned ranges in exclusively runtime-owned
64-KiB slabs, rounded up to the operating system's page size. One allocation is
at most 64 KiB. The process-wide 64-MiB mapped-memory budget includes slab
rounding, prepared copies and staging values; the existing logical resource
quotas remain separate. Operating-system limits can refuse a smaller amount.
Slabs are protected before plaintext is copied or generated into them. Every
released range is zeroized before reuse, and a slab is erased and unmapped when
its last allocation is dropped. Destroying one range cannot unlock another
live range. Allocation and final unmapping share a lock so a still-charged
mapping cannot produce transient budget exhaustion after its last owner exits.

Platform policy:

| Target | Locked memory and dump policy |
|---|---|
| Linux / Android | `mlock`, `MADV_DONTDUMP`, and `MADV_DONTFORK` on each mapping. Both soft and hard `RLIMIT_CORE` become zero; release initialization also sets `PR_SET_DUMPABLE` to zero. |
| macOS / iOS | `mlock`, `VM_INHERIT_NONE`, and `MADV_ZERO_WIRED_PAGES` on each mapping. Both soft and hard `RLIMIT_CORE` become zero. |
| Windows 10 version 1703+ / Server 2016+ | `VirtualAlloc` and `VirtualLock`, with WER exclusion for each mapping and the process's existing WER flags preserved while adding `NOHEAP`. Windows execution qualification remains required. |
| Other targets | Protected allocation is refused; support requires an explicit platform implementation and qualification. |

These are process-local policies. Mesh does not raise memory-lock limits, request
privileged capabilities, change system crash-reporting settings, or modify other
processes. Forked children must execute a new program before using Mesh; secret
mappings are deliberately not inherited, and continuing an inherited runtime
is unsupported. Windows Error Reporting exclusions are not a promise against
administrator/debugger-generated dumps. Locking memory does not protect a
running process from an authorized debugger, a compromised kernel or physical
memory access.

The WER exclusion API's [minimum supported Windows versions](https://learn.microsoft.com/en-us/windows/win32/api/werapi/nf-werapi-werregisterexcludedmemoryblock#requirements)
are a native runtime requirement; older Windows versions cannot load these
direct imports. Release initialization means a Rust runtime compiled without
`debug_assertions` (the normal Cargo release profile), independently of the
Mesh program's optimization level. Build provenance must record both settings.

The guarantee covers the listed runtime allocations, not every temporary copy
inside a cryptographic provider or native host. Provider-owned stack/heap
buffers and initial provider outputs may exist outside locked pages; their
separate zeroization and leak checks still apply. This also does not make normal
Mesh `Bytes` or `String` protected memory. The lack of an allocator fallback
must be exercised on every supported native target with real app workloads and
their normal sandbox, memory-lock limits and release settings before adoption.

Qualification includes actual kernel mapping checks for retained, prepared and
consumed resources and map staging; zeroization/reuse and concurrent owners;
bounded copies; real operating-system denial in an owned subprocess; and a
release-only initialization test before any secret is created. Cross-compilation
or a passing allocator probe alone does not qualify a platform's application.

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

### BR1 resources and crash recovery

`BlindRsaSecretKey` is resource kind 9; `BlindRsaBlindingState` is kind 10.
The public `BlindRsaBlinded` aggregate is affine because its `state` field
contains the inverse blinding factor. The inverse is never a public `Bytes`
value. Key import consumes PKCS#8 DER from `SecretBytes`; the public key is
ordinary canonical SPKI. Finalization consumes a valid owned state on both
success and error. Resource owner, generation, kind and lifetime checks apply
to these resources exactly as to existing secret types.

Storage purpose 18/value-kind 9 seals the validated BR1 issuer PKCS#8 key on
Linux and macOS. Purpose 19/value-kind 10 seals the 256-byte inverse, 32-byte
SHA-256 SPKI binding and 48-byte SHA-384 message binding on every target.
Neither resource has a plaintext export API. A caller persists the sealed
state together with the immutable public request before issuance so a crash
can resume that request. Authenticated storage binds the complete versioned
context and protects confidentiality, not application rollback or replay;
recovery and request idempotency remain the application's responsibility.

Private-key operations and private-key storage return `UnsupportedTarget` on
iOS, Android and Windows; client state storage remains available. Public
compiled-Mesh tests cover key/state restore, context and type mismatch,
tampering and finalization's message binding. Runtime resource misuse and
leak-sentinel evidence remain separate mandatory gates.

### Braid responder state

Development storage purpose 22 accepts only a `SecretMap` with capacity three.
Phases 0 and 1 require exactly the 32-byte `root` and `mac` entries; phase 2
additionally requires the 32-byte `output` entry. The context is the existing
123-byte versioned context followed by one phase byte, with a nonzero session
identifier and snapshot revision. Its authenticated object identifier must bind
the application's remaining protocol state. Existing purposes retain their
123-byte contexts; the separate incremental-KEM continuation uses purpose 21.

The new role validates and stages secret-map material in protected memory.
Restore authenticates the blob before interpreting its private map or
publishing a resource. Sealing revalidates the borrowed map's contents as well
as its owner, kind and generation before publishing ciphertext. Application
code must still atomically commit matching metadata, continuation and transport
state, and reject stale snapshots; sealing alone does not prevent rollback.
This role has focused host qualification and Morse responder dogfooding. It
does not establish a complete sparse post-quantum ratchet or native release.

### Braid session epoch private key

Development purpose 23 stores an existing `MlKemPrivateKey` as its exact
64-byte seed in a 131-byte authenticated blob. Its canonical 123-byte context
requires a nonzero session and positive snapshot revision. The application
object digest must bind the configuration, both account/device identities,
session, role, epoch, substate, exact public key components and transition.
The runtime checks the outer typed context; the application owns those object
semantics and atomic antirollback/account/session checks.

The new role stages seed material and encrypts/decrypts in protected memory;
ordinary outputs contain ciphertext only. Source identity, width and contents
are revalidated after unlocked preparation/reservation and before ciphertext
publication. Authentication precedes private interpretation or handle
publication, and a refused final allocation/quota check publishes no resource.
The existing typed key API and ownership contract are unchanged. Purpose 15
remains the zero-session account prekey; purpose 22 retains exactly its
124-byte context and phases 0/1/2. Older provider temporary-buffer guarantees
are not retroactively strengthened. No new secret exporter, sparse wire mode
or native qualification is implied by this storage purpose.

### Protocol preimages

`HashPreimage` is resource kind 11 and storage purpose 20. It contains exactly
32 operating-system-random bytes intended for eventual protocol publication.
It has no import or conversion from caller bytes or other secret resources.
`HashPreimage.commitment` borrows it to publish SHA256;
`HashPreimage.reveal` consumes it to deliberately publish the original bytes.
This narrow disclosure is not a generic secret export. Existing private keys
and ratchet material cannot be passed to it. Before disclosure it follows the
same ownership, zeroizing storage, quota and cleanup rules as other resources.
After disclosure ordinary public-data semantics apply. Restoring a sealed copy
permits repeated disclosure for recovery, so applications must independently
persist authorization and enforce payment state transitions. See
[`mesh-lightning`](../../packages/mesh-lightning/README.md).

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


## Bounded speculative maps

`SecretMap.fork(borrow map)` returns an independent actor-owned map with the
same capacity and entries, or a typed resource-limit error. Both maps retain
affine ownership and zeroizing storage. Mutating or destroying the candidate
does not alter the original. Use a fork to prepare a transition, authenticate
it, then replace the committed map; discard the candidate on failure. Forking
deliberately retains another copy until that candidate is consumed or dropped.
Cleanup skips cleared nested enum payloads after a move, including inside
`Option` and `Result`; it never dereferences their null sentinels.

Service calls and casts copy top-level string arguments into owned mailbox data
before the sender can exit. Receive relocates those strings into the receiver's
heap. String replies use the same ownership transfer. This prevents service
state (for example a rate-limit map key) from referencing a finished HTTP request
actor's heap. The SQLite todo runtime test exercises this across independent
HTTP requests. This change does not claim deep copying of arbitrary aggregates
containing pointers.
