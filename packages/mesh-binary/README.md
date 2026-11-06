# mesh-binary

Bounded canonical binary encoding and immutable decoding for Mesh protocol packages.

`BytesBuilder` is an affine, mutable runtime builder capped by its declared
limit and a 64 KiB hard ceiling. Writes borrow it; `finish` consumes it.

`Binary.Reader.read_vector` uses a canonical unsigned 32-bit big-endian length
prefix. Readers reject invalid state, input beyond the declared total limit,
truncation, per-vector limit violations, and trailing bytes.

`Binary.Erasure.encode_erasure_chunk(message, width, index)` returns one
systematic or repair word. `decode_erasure_chunks(message_bytes, width,
indices, payloads)` returns `None` below the required independent rank and the
original bytes when reconstruction and all consistency checks succeed.

The field is GF(2^16) with polynomial modulus `0x1100b` and two-byte big-endian
symbols. Messages contain 1–1152 bytes; payload width is even and 32–256 bytes.
Source word i is the polynomial value at field element i, so the first k words
are the original zero-padded source. Indices are 0–65535; negative indices and
65536 are refused without wrap. The decoder validates at most 72 supplied
entries before deduplication, including every length and index. Identical
index/payload pairs are redundant; distinct indices with identical payloads
still contribute independent rank. Conflicting duplicates, inconsistent
surplus words and nonzero reconstructed padding are errors.

`InvalidParameters` covers message/width parameters and encoder index;
`MalformedChunk` covers decoder counts, lengths and indices;
`ConflictingDuplicate` and `InconsistentCodewords` distinguish the remaining
refusals. The implementation uses fixed scalar field arithmetic and bounded
Lagrange prefix/suffix products. It has no field lookup table, recursion, host
callback, network or storage dependency. Its largest retained field matrix is
690 cells, rather than the reference decoder's augmented matrix. The padded
message can reach 1380 bytes at size 1152 and width 230. Input payloads are at
most 18,432 bytes. Prefix and suffix lists have at most 37 values each; only the
selected k payloads become field rows. Reconstruction performs at most 72 word
evaluations, including surplus consistency checks. Every range and temporary
list derives from validated k, width or supplied count.

For the current 64-bit Mesh collection layout, a conservative per-call object
allocation bound is 16 MiB, including input data, list backing capacities,
headers, closure/range objects, `Bytes.slice` copies, prefix append copies, and
even discarded intermediate objects if GC retains them until return. This
budget allows 24 KiB for logical supplied inputs, 512 KiB for setup/reconstruction
and 128 KiB for each of at most 72 evaluations (under 10 MiB in total, with the
remainder as margin). Each evaluation's allowance covers 64 KiB of cumulative
prefix/suffix copies, 32 KiB of ranges/reduction closures, 16 KiB of builder,
write-result and loop objects, and 16 KiB of other word/list copies. Encoding
needs only one such evaluation. This is a static upper bound for codec-created
objects, not a measured exact live heap, an allocator/RSS quota, or a bound on a
caller's preexisting oversized list backing store. Runtime pages, retained GC
arenas and process startup memory are separate. Native integration still needs
its own maximum-rank timing and memory qualification.

Erasure rank and consistent reconstruction do not authenticate a message.
This field/evaluation convention does not claim RFC or Signal wire
interoperability. Callers must authenticate parameters, indices and recovered
public bytes independently; no transport scheduler or epoch policy is provided.
