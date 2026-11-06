# GC value layout

Mesh's actor collector conservatively scans aligned machine words in live stack,
register and heap storage. A compiler representation containing heap pointers
must preserve each pointer as a complete aligned word across calls, aggregate
copies, closure capture and actor suspension.

Mixed sum types use a one-byte tag followed by aligned 64-bit payload words.
Variant overlays reserve a full word for the tag before their first payload
field. This matters for Boolean payloads and for a sum stored immediately after
a Boolean in an enclosing struct: neither may put a live pointer in unaligned
storage or in padding that an aggregate copy discards. Constructors initialize
unused payload bits before narrow fields are written.

Pointer-only `Result` and `Option` retain their existing tag/pointer layout and
runtime offsets. Arbitrary structs and sums are compiler-internal layouts, not a
stable native FFI format; rebuild all Mesh artifacts together when this layout
changes. Scalar native ABI support does not imply that a host can reinterpret a
sum's memory.

The regression `gc_mixed_sum_payload.mpl` keeps boxed records, Boolean/scalar
variants and multiple-field variants alive across collection pressure in both
unoptimized and optimized builds. The actor, resource, pattern and native library
suites additionally cover copying, destruction and runtime boundaries. Release
qualification still requires the repository's authoritative verification gates.
