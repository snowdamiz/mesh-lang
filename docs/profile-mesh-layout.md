# Build a Mesh executable with an observed section order

Use `scripts/mesh-section-layout.py` to try an observed section order on a GNU
AArch64 Linux executable. The tool requires Python 3.9 or later, the target
Mesh runtime archive, and a C compiler driver that uses LLD. Mesh still
builds and links the package normally. When the profile or linker is
incompatible, the build keeps the original executable and records the reason.

## Build the stock executable and record a representative run

Run the commands from the repository root. Set `RUNTIME` to the GNU AArch64
archive that you pass to your ordinary Mesh build. Use the filename
`libmesh_rt.a`, which can be a symlink. Use a new path for each build receipt.

```sh
tool=scripts/mesh-section-layout.py
python3 "$tool" build --runtime "$RUNTIME" --cc-driver clang \
	--cc-arg=-fuse-ld=lld --cc-arg=-Wl,-z,now \
	--receipt stock.json -- ./my-package --output ./my-app-stock
python3 "$tool" record --elf ./my-app-stock --map stock.stock.map \
	--receipt stock.json --output observed.json --runs 8 -- --my-app-argument
```

Keep the release linker flags you use for the application. The layout
requires a dynamic GNU AArch64 executable with section GC, RELRO, immediate
binding, and 64 KiB ELF segment alignment. Pass any extra driver flags with
`--cc-arg`. With Zig, use `--zig --cc-driver zig --cc-arg=cc` and pass your
normal target and linker flags as additional `--cc-arg` options. Zig builds
the stock executable first. The tool requires its direct LLD relink to match
that executable byte for byte.

`record` runs the stock executable on Linux with the arguments after `--`.
Arrange any input files or environment variables that the application needs.
The recorder reads file-backed pages at process exit. Those pages include
kernel fault-around pages. Page presence does not identify the instructions
that ran. Record runs that complete successfully. Use `--observer` to supply
a previously built copy of `scripts/lib/mesh_section_observe.c` if your
recording machine has no C compiler.

## Train and build with the profile

```sh
python3 "$tool" train --runtime "$RUNTIME" --elf ./my-app-stock \
	--map stock.stock.map --observations observed.json \
	--output sections.json --report coverage.json
python3 "$tool" build --runtime "$RUNTIME" --cc-driver clang \
	--cc-arg=-fuse-ld=lld --cc-arg=-Wl,-z,now \
	--profile sections.json --receipt profiled.json \
	-- ./my-package --output ./my-app-profiled
```

For an existing collector, provide the same JSON format as `record`.
Include `schema` equal to `1`, `elf_sha256`, `map_sha256`, the count
`observations`, and `file_ranges`. Each range contains a file offset, byte
length, and observation count. Sort the ranges by file offset. Adjacent ranges may touch, and ranges cannot
overlap. The count for a range cannot exceed
`observations`. Use the digests from the matched stock build receipt.

The profile contains runtime archive member names, input section names,
sizes, and alignments. You can reuse it when you move the build or change
the package. The link receipt reports how many entries still match and how
many differ or are absent. Only unambiguous live sections participate.
`coverage.json` accounts for recorded bytes from application objects,
other archives, and inputs that cannot be moved by the profile.

Check that `profiled.json` reports `"state": "candidate"` before comparing
performance. Compare against the stock executable using a workload or run
that you left out of training. To use the normal build directly, run
`meshc build` with your normal options.
