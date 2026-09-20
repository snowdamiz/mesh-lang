# mesh-binary

Bounded canonical binary encoding and immutable decoding for Mesh protocol packages.

`BytesBuilder` is an affine, mutable runtime builder capped by its declared
limit and a 64 KiB hard ceiling. Writes borrow it; `finish` consumes it.

`Binary.Reader.read_vector` uses a canonical unsigned 32-bit big-endian length
prefix. Readers reject invalid state, input beyond the declared total limit,
truncation, per-vector limit violations, and trailing bytes.
