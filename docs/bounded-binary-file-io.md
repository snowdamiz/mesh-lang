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
