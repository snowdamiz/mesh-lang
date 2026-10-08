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

## Swap and core dumps

The table keeps every stored secret in a pool of 64 KiB chunks mapped only
for secrets (`compiler/mesh-rt/src/secret_memory.rs`), not on the ordinary
heap. Stored bytes are copied in, packed in 32-byte units, and zeroized
before their units go back to the pool; a chunk that empties is unlocked and
unmapped, except the first. Each chunk is locked into RAM when it is mapped
and, where the OS has a flag for it, excluded from core dumps. Locking never
stops a secret from working: a chunk the kernel will not lock is still used,
and if no chunk can be mapped the secret stays on the heap. Each secret
placed outside locked memory adds one to the process-wide counter returned by
`mesh_secret_unlocked_count()` (C ABI; zero means every secret was locked).

Release executables turn core dumps off before `main` runs: `meshc build
--opt-level 2` (and above) makes the generated `main` call
`mesh_rt_disable_core_dumps()`, which sets the core size limit to 0 (soft and
hard, so nothing later in the process can raise it) and, on Linux and
Android, makes the process non-dumpable. Debug builds (`--opt-level 0` and
`1`, the default) and `meshc test` leave both alone, so they can still be
debugged and dump cores. A Mesh library embedded in a host app (mobile,
desktop) has no generated `main`; the host calls
`mesh_rt_disable_core_dumps()` itself if it wants the same.

What each OS guarantees:

| OS | Kept out of swap | Kept out of core dumps | Limits and gaps |
|---|---|---|---|
| Linux | `mlock` on each chunk | `MADV_DONTDUMP` on each chunk (Linux 3.4+); release executables set `RLIMIT_CORE` to 0 and `PR_SET_DUMPABLE` to 0, so the kernel writes no core at all, including to a `core_pattern` pipe such as systemd-coredump | `RLIMIT_MEMLOCK` bounds locking: the kernel default is 64 KiB before Linux 5.16 and 8 MiB from 5.16; `CAP_IPC_LOCK` lifts it. A 64 KiB limit locks at most one chunk (2,048 32-byte units). A non-dumpable release process also refuses `ptrace` from other processes of the same user. Hibernation writes locked pages to the hibernation image |
| Android | `mlock` (bionic), so not compressed into zram (Android swaps to compressed RAM, not storage) | `MADV_DONTDUMP` | App processes inherit `RLIMIT_MEMLOCK` from zygote: the kernel default (64 KiB before Linux 5.16, 8 MiB from 5.16) unless the device's init changes it. Apps run the runtime as a library, so the host decides on `mesh_rt_disable_core_dumps()`; non-debuggable app processes are already non-dumpable. debuggerd tombstones still read the crashed process through ptrace: they include registers, stack and a few hundred bytes around each register-held address, so a crash while a secret is in use can put part of it in a tombstone (readable by the system, not by other apps) |
| macOS | `mlock` | No per-region exclusion exists: XNU's core writer dumps every readable region except IOKit and device-pager memory (`bsd/kern/kern_core.c`, `coredumpok()`), wired pages included. Release executables set `RLIMIT_CORE` to 0 (the default soft limit is already 0, and a core also needs `kern.coredump=1` and a writable `/cores`) | Wiring is bounded by `RLIMIT_MEMLOCK` (unlimited by default; XNU enforces it) and `vm.user_wire_limit` (about three quarters of RAM on a 16 GB development Mac). Crash reports (`.ips`) hold backtraces and registers, not memory. Swap files are encrypted by the OS; locked pages never reach them |
| iOS | `mlock` where the kernel allows it; a refused lock is counted | Apps produce no core files; crash reports hold backtraces and registers only | No swap to storage (compressed memory only). The kernel sets the wire limit; the counter shows how many secrets were not locked. The host decides on `mesh_rt_disable_core_dumps()` |
| Windows | `VirtualLock` | No region flag; Windows Error Reporting's default minidumps leave heap and pool memory out, but a full dump configured by an administrator (`LocalDumps`, `DumpType` 2) includes it | `VirtualLock` is bounded by the process's minimum working set (a few hundred KiB by default), so one or two chunks lock and the rest are counted. `mesh_rt_disable_core_dumps()` does nothing |
| Other | Nothing | Nothing | Secrets stay on the heap and are counted |

These protect the copies the table holds. They do not cover transient copies:
while a runtime call uses a secret, it works on a zeroizing heap copy taken
out of the table (`with_owned_resource`), providers keep stack temporaries
(libcrux, for one, wipes nothing), and a consumed resource is handed to its
caller as a zeroizing heap copy. Those copies live for one call and are
zeroized when dropped, but they are neither locked nor excluded from dumps. A
forked child would get a copy-on-write copy of the pool; the runtime never
forks without `exec`.

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

`StorageKey.from_secret(material, context)` derives a storage key from a secret
someone holds, so that sealed private keys can be restored elsewhere from that
secret alone (an account recovered from a backup's recovery code). It consumes
the `SecretBytes` (at least 16 bytes) on every path and computes
`HKDF-SHA-256(material, "mesh/storage-key/derived/v1", context)` for a 1- to
256-byte context that separates uses. The derived key is an ordinary
`StorageKey` resource: every `*.seal_for_storage` and `unseal_from_storage`
takes it, with the same contexts, purposes, and blob format. A derived key has
no durable counter, and every derivation of the same secret yields the same
key, so each derivation draws a random 4-byte nonce prefix and a random
starting counter below 2^63 from the OS; nonces of two derivations coincide
with probability about 2^-95 per seal, the bound of random 96-bit nonces. Its
protection is the secret's: whoever holds the secret can open what it sealed.

`Secret.from_bytes(value)` takes in material that reached the program as
ordinary `Bytes`, such as a code a person typed, as `SecretBytes`. It exists
because such material has no other way in; it does not erase the source
`Bytes`, which stays in garbage-collected memory until collected, so it is for
input that was never a secret the runtime held. Nothing turns `SecretBytes`
back into `Bytes`.
Storage context purpose `15` is reserved for ML-KEM prekey seeds and accepts
only `MlKemPrivateKey`; resource kind `8` is used because value-kind `7` already
identifies ordinary sealed bytes.

Blind RSA adds two resource kinds. Kind `9` is `BlindRsaSecretKey`: a 2,048-bit
private key held as canonical PKCS#8 DER (1,190 to 1,220 bytes), created only
by `Crypto.blind_rsa_generate` or by consuming `SecretBytes` in
`Crypto.blind_rsa_from_secret`, and never exposed as `Bytes`. Storage purpose
`18` (blind RSA issuer key) accepts only kind `9`, requires an all-zero session
ID and enforces the same length rule. Kind `10` is `BlindRsaBlindingState`: the
256-byte inverse of the blinding factor, which must stay secret for issuance to
stay unlinkable. It lives inside the affine `BlindRsaBlinded` value, and
`Crypto.blind_rsa_finalize` consumes it on every path, including failures;
kind `10` has no storage purpose. On targets without a signing provider (iOS,
Android, Windows) kind `9` is never created and sealing or unsealing it returns
`UnsupportedTarget`.

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
