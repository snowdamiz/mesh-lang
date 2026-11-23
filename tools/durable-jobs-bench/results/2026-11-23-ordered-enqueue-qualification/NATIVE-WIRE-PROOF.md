# Recheck the native truncated-response test

The qualified producer artifact is already admitted for matched timing.
The native unit-test result remains open in the product adoption record.
The compiled producer passed the separate native disconnect and rollback
checks. The same source's truncated-response test passed on Darwin arm64.
The three native unit-test attempts and all producer attempts are archived.
[Native results](NATIVE.md)

A later proof-only run can reuse the first retry's source copy and
release Cargo cache. The source copy has the test vectors, with identical
bytes at stock `3f69b150` and candidate `77c447f7`. The three native
attempts ended at missing inputs, a 190-second release test compile limit,
and a shorter compile limit after changing only the test profile. Allow
about ten minutes for a separate SETUP slot. An eight-minute command
budget leaves time to export evidence and release locks. The first
runtime build took about three minutes. The first retry reached its 190-second limit before Cargo ran the test.
[Vector checksums](wire-vectors.sha256)

The matrix owner allocates the new reservation. Acquire the local timing
lock first, then the remote active-arm lock and native lock. With the
locks held, set `NATIVE_WIRE_GRANT_END` to the published UTC end as a
Unix timestamp. Run these commands on the same pinned host. Redirect the complete
output into a fresh receipt. Record the exact command exit and verify
the executable before and after the command.

```bash
base=/var/lib/durable-jobs-bench/native
setup="$base/setup/ordered-enqueue-v1"
proof="$setup/wire-source-wire-test-retry-1894ded3c5504623ad57c80cbaa3a8fe"
source="$base/source-ordered-enqueue-v1"
mesh="$base/builds/mesh-ordered-enqueue-v1/mesh"
llvm="$base/toolchain/llvm-21.1.8"
rust="$base/toolchain/rust-1.99.0"

cd "$proof"
diff -r "$source/compiler" "$proof/compiler"
diff -r "$source/.cargo" "$proof/.cargo"
cmp "$source/Cargo.lock" "$proof/Cargo.lock"
cmp "$source/Cargo.toml" "$proof/Cargo.toml"
test "$(cat NATIVE_MESH_REVISION)" = 77c447f773a4d0af5c5578873a2410301b3ba069
printf '%s  vectors.sha256\n' '52c03f05fb23775a16866433bf7aa178398471698d2830df64d8b5884357469f' | sha256sum -c -
sha256sum -c vectors.sha256
printf '%s  %s\n' 33ebb48f27c4e1623b5019400847812deb48ca6a4dba26b594efb0587a301954 "$mesh" | sha256sum -c -

: "${NATIVE_WIRE_GRANT_END:?set the published grant end as a Unix timestamp}"
test "$((NATIVE_WIRE_GRANT_END - $(date +%s)))" -ge 540
wire_code=0
timeout --signal=TERM --kill-after=10 480 env -i \
    "PATH=$rust/bin:$llvm/bin:/usr/bin:/bin" LC_ALL=C.UTF-8 \
    "LLVM_SYS_211_PREFIX=$llvm" "CARGO_HOME=$setup/cargo-home" \
    "CARGO_TARGET_DIR=$setup/cargo" "TMPDIR=$setup/tmp" CARGO_BUILD_JOBS=4 \
    "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$proof/scripts/ci-x86_64-linux-gnu-clang" \
    cargo test --offline --locked --release -p mesh-rt --lib \
    db::pg::tests::ordered_enqueue_reads_a_backpressured_window_and_retires_a_truncated_connection \
    -- --exact || wire_code=$?
printf '%s  %s\n' 33ebb48f27c4e1623b5019400847812deb48ca6a4dba26b594efb0587a301954 "$mesh" | sha256sum -c -
printf 'wire_exit=%s\n' "$wire_code"
test "$wire_code" -eq 0
```

Accept the result when Cargo names the expected test with `ok`, reports
one passed test, and both executable hash checks agree. Archive the log
and exit, including any timeout. Keep the compiler command separate from
the existing producer build and executable. After exporting the receipt,
release all three locks. The proof source copies and matching Cargo
cache are retained on the host for this reservation.
