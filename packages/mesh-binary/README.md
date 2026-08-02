# mesh-binary

Bounded immutable binary decoding for Mesh protocol packages.

`Binary.Reader.read_vector` uses a canonical unsigned 32-bit big-endian length
prefix. Readers reject invalid state, input beyond the declared total limit,
truncation, per-vector limit violations, and trailing bytes.
