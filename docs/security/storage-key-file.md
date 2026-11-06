# Durable filesystem storage keys

`StorageKey.file(directory: String) -> StorageKey!CryptoError` opens a persistent,
actor-owned wrapping capability on Linux and macOS. The flat alias is
`storage_key_file`. Other targets return `CryptoError.UnsupportedTarget`; this
API does not select a weaker or ephemeral fallback. Mesh code cannot export
the wrapping material. No native application callbacks are required.

## Directory contract

Supply an absolute UTF-8 path of at most 4096 bytes, with at most 64 components
of at most 255 bytes each. Empty components, trailing slashes, `.` and `..`, NUL,
and symlinks in any component are rejected. Resolve an application's trusted
configuration path before passing it to this API. The parent directories must
already exist; the runtime can create the final component.

The final directory must belong to the effective user and have exactly `0700`
permissions. Existing permissions are checked, never repaired automatically.
The runtime uses `key` and `lock` files inside it. Each must be a regular,
effective-user-owned, single-link file with exactly `0600` permissions. The
lock file is empty and retains its inode throughout normal operation. The key
record is exactly 44 bytes: the existing internal 36-byte wrapping material
(32-byte key and four-byte nonce prefix), then the next counter as a big-endian
`u64`. This is an internal storage record, not a new ciphertext format or a
Mesh-visible import/export interface.

Unsafe paths, ownership, permissions, file types, link counts, truncated or
oversized records, missing records in an initialized store, and missing locks
beside an existing key fail closed with `CryptoError.InternalFailure`. Filesystem
errors retain the OS status in that error's numeric detail; no key or path is
included in the error. Counter exhaustion refuses new sealing operations.

## Reservation and lifetime

Every open and counter reservation uses a process mutex and an exclusive
advisory lock. Opening additionally locks the directory while establishing the
initial record, so concurrent first opens share one key. Reservations lock the
separate, stable lock file; renaming the key record never replaces that lock.
Each operation checks the open files' identity against their directory entries.
Reservations also confirm the directory still occupies the configured path and
the current key material matches the capability's original material.

A reservation reads the current record, increments its counter with overflow
checking, writes a new private temporary file, synchronizes it, atomically
renames it onto `key`, and synchronizes the directory before returning the old
counter. macOS additionally uses `F_FULLFSYNC` on the record before and after
the rename. Failure yields no ciphertext. A counter whose commit completed is
burned even if the process exits before encryption or output. The nonce and
AEAD blob encoding are the existing storage-wrapping encoding.

The resource table holds an `Arc` to a runtime-owned reservation source. An
unlocked reservation retains its own reference, so actor exit cannot free its
file descriptors during I/O. Finishing that reservation releases the reference;
destroying the last capability/preparation closes both descriptors and zeroizes
the retained material. Resource ownership and generation are revalidated before
any ciphertext is returned. Host callback storage keys retain their existing
ABI and lifetime contract.

## Operations and recovery boundaries

Use a local filesystem with working advisory locks, atomic same-directory
rename and durable synchronization. Network filesystems, shared-volume lock
emulation and hardware that lies about durable flushes are not covered.

Never restore or replace a used key directory with an older snapshot: a local
filesystem cannot detect rollback of both its key and counter. Keep the
directory outside application database rollback and restore procedures.
Administrators and processes with the same UID can read or replace this master
key; filesystem permissions do not protect against that authority. This API is
not a hardware-backed secret store.

A crash before the first record commits can leave an empty lock and no key.
That state is deliberately refused rather than guessed to be unused. A missing
record in an initialized store must be investigated and restored through an
operator-controlled recovery procedure; the runtime never deletes the lock or
silently regenerates material. If no capability or sealed data was ever issued,
configure a fresh empty private directory. Temporary `.key-*` files can remain
after a crash; they are never loaded as the authoritative record. Do not promote
one manually or delete an active store as an automatic repair.

## Evidence and limits

Runtime tests exercise reopening, concurrent threads and processes starting
with an absent directory, unique reservations, actor-exit source lifetime,
unsafe modes, symlinks, FIFOs, hard links, record/lock replacement, counter
exhaustion and malformed lengths. Subprocess tests use `SIGKILL` after file sync,
after rename, after directory sync, and immediately before returning a counter.
These are process-crash tests, not physical power-loss or filesystem fault
certification. The compiled Mesh restart proof seals and restores both a BR1
issuer key and its pending blinding state using the public API in separate
executions.

The implementation follows the descriptor-relative, no-follow operations in
the [Linux open documentation](https://man7.org/linux/man-pages/man2/open.2.html),
the directory synchronization requirement in
[fsync](https://man7.org/linux/man-pages/man2/fsync.2.html), and Apple's distinction
between `fsync` and drive-cache flushing in
[fcntl](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fcntl.2.html).
This local validation does not constitute external security review or release
approval.
