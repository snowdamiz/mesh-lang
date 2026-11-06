# Bounded Binary File I/O

Mesh exposes binary range I/O for native protocol cores that must not route
plaintext files through a string or host-language bridge:

```mesh
let size = File.size(path) ?
let chunk = File.read_bytes(path, offset, length) ?
File.write_bytes(destination, offset, chunk, offset == 0) ?
```

`File.read_bytes` reads at most 65,536 bytes and may return fewer bytes at end
of file. `File.write_bytes` accepts at most 65,536 bytes. Both reject negative
offsets and ranges beyond 16 MiB. A truncating write is valid only at offset
zero, making the first sequential write explicitly replace an old destination.
`File.size` rejects missing paths, directories, and sizes that do not fit in a
Mesh `Int`.

The 16 MiB ceiling is the reviewed messenger attachment and backup profile.
Larger files require a versioned quota API rather than silently raising this
bound. Each call is synchronous; callers provide progress and cancellation by
performing one range operation at a time.

Version 2 supplies that explicit quota:

```mesh
let chunk = File.read_bytes_v2(path, offset, length, maximum_size) ?
File.write_bytes_v2(destination, offset, chunk, offset == 0, maximum_size) ?
```

`maximum_size` must be a positive public byte limit of at most 1 GiB. The
requested range must fit within it, and each call still transfers at most
65,536 bytes. Negative offsets, zero lengths, overflow, and truncation at a
nonzero offset are refused before opening the path. After opening, the same
descriptor must name a regular file whose observed size fits the quota.
An oversized destination is refused before truncation or a data write.
Reads can return fewer bytes at EOF. The original APIs retain their 16 MiB
limit. The free spellings are `file_read_bytes_v2` and `file_write_bytes_v2`.

The quota is a resource bound, not filesystem authorization or an atomic
snapshot. Concurrent writers are not serialized. Ordinary filesystem errors
can occur after an admitted operation starts, and a failed write may be
partial. All arguments and caller control flow retain the ordinary File
privacy restrictions; the quota does not authorize plaintext disclosure.

On Unix, version-2 opens use nonblocking mode so a FIFO without a peer can
be refused without waiting for that peer; the same-descriptor regular-file
check still applies. Non-Unix platforms use portable open behavior. This is
not a general I/O deadline: path lookup, device or filesystem operations and
subsequent I/O may still wait. No atomic snapshot or concurrent-writer
serialization is implied.
