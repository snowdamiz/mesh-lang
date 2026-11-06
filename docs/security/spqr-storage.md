# Typed SPQR map storage

The existing borrowed `SecretMap.seal_for_storage` and
`SecretMap.unseal_from_storage` APIs support two separate storage purposes:
24 (`SpqrChains`, capacity exactly 7) and 25 (`SpqrSkippedKeys`, capacity
exactly 64). Both restore the existing opaque `SecretMap` resource. No secret
export, builtin signature, resource kind, global map capacity, or storage
envelope is added or reinterpreted. Purposes 12, 21, 22 and 23 retain their
existing grammars and provider paths.

These purposes validate supplied authenticated metadata and protected material.
They do not establish KDF provenance, output ownership, coordinator authority,
durable atomicity, or a release/transport activation. An authorized caller can
still select an existing permissive map purpose; an SPQR consumer must select
only the two typed purposes for its SPQR maps.

## Canonical context

All integers use unsigned big-endian encoding. The unchanged 123-byte prefix is
version 1, account (32), device (16), session (32), object (32), purpose (u16),
revision (u64). The session and revision must be nonzero for these purposes.
The extension is:

| Field | Bytes | Rule |
| --- | ---: | --- |
| Extension version | 1 | Exactly 1 |
| Initial local direction | 1 | 0=A2B, 1=B2A |
| Incorporated epoch L, knowledge K | 16 | K<=L and L-K<=1 |
| Transition, descriptor digest | 64 | Two exact 32-byte public fields |
| Slot count | 1 | 1..3 |
| Slots | 26 each | Canonical sequence below |
| Skipped count | 1 | 0..64 |
| Skipped pairs | 16 each | Epoch u64, number u64 |

Each slot contains epoch u64, send-present u8, send-count u64,
receive-present u8 and receive-count u64. Booleans are exactly 0 or 1.
Slots are exactly the contiguous sorted interval `[max(0,K-1),L]`.
Receive-present is always true. Send-present is required at epochs >=K;
an absent older send has count zero. Both counters above K are zero. At most
one send-present slot has a positive send count. Terminal counters/epochs at
u64 maximum are valid saved metadata; the consumer must refuse advancement
rather than wrap.

Skipped pairs are strictly increasing and unique. Every pair refers to a
retained receive-present slot and satisfies `1<=n<receive_count`. The selected
message number is never a cached skipped key. Counts, alignment, exact parser
completion and every invariant are checked. Contexts are 233..1309 bytes;
epoch zero is valid SPQR initialization.

The actor boundary copies public context bytes before a nonce callback. The
existing 123-byte/role22-124-byte copier runs first. Only purposes 24/25 receive
the bounded 1309-byte fallback. Non-SPQR failures return the exact original
legacy error. An over-limit new-purpose ABI input therefore retains the legacy
maximum-length error (expected 123); direct new-role validation reports its
own 1309-byte bound. Local-data byte storage does not use the fallback.

## Protected map grammar

The existing encoding is capacity u16, entry count u16, then each entry's
name-length u16, value-length u32, name and value. Values are exactly 32 bytes.
Names must equal the entire unique set declared by the metadata; no extra,
duplicate, omitted, malformed or trailing material is accepted. Map insertion
order is arbitrary. Metadata order is canonical.

| Purpose | Exact names | Accepted encoded bytes | Wrapped maximum |
| --- | --- | ---: | ---: |
| 24, chains | Root `[0]`; `[1] || epoch_u64 || global_direction_u8` for every present direction | <=331 | 398 |
| 25, skipped | `[2] || epoch_u64 || number_u64` for every declared pair | <=3524 | 3591 |

Local send uses the stable initial direction and receive its opposite; Braid
role alternation does not change these names. Initialized chains contain root
and both epoch-zero directions: 139 encoded bytes, 206 wrapped. An empty skipped
map is 4 encoded bytes, 71 wrapped. The validators use fixed metadata arrays,
direct name slices and a bounded seen mask. The 64-entry mask has an explicit
full-width case; no implicit shift or index wrap is used. Private values are
never decoded into a generic map representation or an ordinary temporary.

## Reservation, authentication and publication

Seal validates copied public metadata, checks the source's exact expected
encoded length, and allocates that bounded protected copy outside the global
resource-table lock. It rechecks owner, generation, kind and length before
copying. The completed protected copy is the source capture point. Values
before that capture point are not claimed to have been independently pinned.
Allocation failure preserves the original borrowed resource.

The wrapping key and captured map are prepared before the actual host nonce
reservation. No process or table guard survives the callback. The returned
counter is committed as spent first. The same captured map identity and its
complete protected bytes are checked before AEAD, and both map/key identities
and map bytes are checked again after AEAD, before returning ciphertext.
Rebinding a caller handle pointer to a replacement generation cannot authorize
the pending old borrow. Failures publish no ciphertext; spent host counters do
not roll back. A retry uses a strictly newer counter.

Open checks public framing and binding, authenticates directly into protected
memory, then validates context and exact private schema. It revalidates the
wrapping-key identity before publishing one actor-owned map. Authentication,
schema and ownership failures publish no resource. Internal generic sealing
also routes these purposes through protected AEAD; internal generic opening
refuses them before constructing an ordinary plaintext owner.

The storage envelope remains version 1, algorithm u16, nonce 12, binding 32,
ciphertext length u32, ciphertext and tag 16 (67 bytes overhead). Existing
storage-wrap domain, AAD and nonce construction are unchanged.

## Bounded storage

Public context copying is capped at 1309 bytes. Metadata contains only three
slots and 64 public pairs. Validation scans at most 64 entries and at most 64
declared skip names per entry; it allocates no private lookup table.

Sealing adds at most two protected map buffers of 3524 bytes plus a 36-byte
protected wrapping-key preparation: 7084 bytes of additional protected payload.
Including the already owned maximal skipped map/key gives 10644 protected
payload bytes. The chain role uses at most 331 bytes per map buffer. Fixed
zeroizing AEAD key arrays and cipher state remain stack temporaries as in the
existing detached storage roles. Mapping/page overhead and allocator bookkeeping
are additional; these payload bounds are not process RSS limits.

Opening stages at most 3524 bytes of protected plaintext (the union role bound
preserves binding-before-schema error priority) plus its 36-byte key copy. It
moves the validated protected allocation into the resource table. The shared
ABI still copies public ciphertext under the legacy 65603-byte limit before
role dispatch; oversized SPQR frames are rejected before protected allocation.

## Consumer binding

A consumer can bind the full canonical scope and transition through distinct
component object domains:

`SHA256(ASCII("mesh/spqr/storage/v1/chains" or ".../skipped") || 0x00 ||
canonical_scope || initial_direction_u8 || revision_u64 || transition32 ||
descriptor_digest32)`.

Canonical scope is protocol-info length u8 and 1..128 bytes, configuration32,
account32, device16, peer-account32, peer-device16 and session32. A scheduler's
descriptor digest hashes its canonical metadata with component-hash fields
zeroed to avoid a hash cycle. The runtime authenticates supplied bindings; the
coordinator must derive them from captured authenticated scope and commit all
components in its original transaction. These storage purposes alone provide
no same-CAS or exactly-once authority and persist no raw epoch output.
