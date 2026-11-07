# Reproduction commands — candidate r2

Working directory for every command: `/Users/andreyyurlov/Documents/dev/mesh-lang`.

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
export CARGO_TARGET_DIR=/Users/andreyyurlov/Documents/dev/mesh-lang/.tmp/native-owned-invocations/target
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export LLVM_SYS_211_PREFIX=/opt/homebrew/opt/llvm@21 CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=3
export MESH_RT_LIB_PATH=$CARGO_TARGET_DIR/debug/libmesh_rt.a
export MESH_TEST_RT_LIB_PATH=$CARGO_TARGET_DIR/debug/libmesh_test_rt.a
cargo test --locked --workspace --lib --bins
cargo test --locked -p mesh-parser -p mesh-typeck -p mesh-codegen -p mesh-fmt -p mesh-lint
cargo test --locked -p mesh-rt --lib library::invocation::tests
.tmp/native-owned-invocations/target/debug/deps/mesh_rt-3bf9f93c3f2f7781
cargo test --locked -p mesh-rt --test library --test library_boundaries --test library_gc --test library_memory --test http_response_persistence --test http_accept_cooperative
cargo test --locked -p meshc --test e2e_native_invocations --test e2e_library
cargo fmt --all -- --check
git diff --check
```

The generated tests rebuild the current runtime/test-runtime static archives before invoking the actual compiler. They set scheduler min/max to1 for their native Python hosts and use only owned loopback. Exact resulting family is `generated-abi-1794018547245341000`, pinned in `final-family-r2.json`; do not overwrite it for reproduction. A new test invocation creates a new timestamped family. The runtime unit binary was built by the focused cargo command immediately before its full run; it includes the new service/descendant test absent from the earlier workspace binary run. The20 ignored unit tests are unchanged baseline coverage: real PostgreSQL/TLS database prerequisites, host-sensitive release timing/OpenSSL gates, and the FIFO subprocess helper. The all-crate nonlibrary e2e/release/fuzz/cross-platform CI matrix was not run as part of this bounded local unit; no full release/CI claim.

Meaningful regression logs remain separate: callback-compat-red, txn-http-unwind-red, room-refusal-red, peer-control-red, capture-lifetime-red, companion-header-red, repo-refusal-red, root-completion-red. `workspace-lib-bins2.log` captures the missing JIT symbol; `jit-submit-green2.log` fixes it. The first expanded pool positive test needed the local fake PostgreSQL server to answer extended-protocol Sync; that fixture error and its correction are recorded by workspace runs3/4. Failed logs remain retained, not counted as passes.

Actual app source was copied by `stage_core.py` into `core-bootstrap/`, with hashes and parent app HEAD in `core-bootstrap/staging.json`. Only scratch manifest dependency paths changed. Actual176 operation builds:

```sh
MESH_RT_LIB_PATH=/Users/andreyyurlov/Documents/dev/mesh-lang/.tmp/native-effect-authority/target/debug/libmesh_rt.a MESH_TEST_RT_LIB_PATH=/Users/andreyyurlov/Documents/dev/mesh-lang/.tmp/native-effect-authority/target/debug/libmesh_test_rt.a .tmp/native-effect-authority/target/debug/meshc build .tmp/native-owned-invocations/core-bootstrap/mobile-core --artifact cdylib --opt-level 0 --output .tmp/native-owned-invocations/app-baseline/libmessenger_mobile.dylib
.tmp/native-owned-invocations/target/debug/meshc build .tmp/native-owned-invocations/core-bootstrap/mobile-core --artifact cdylib --opt-level 0 --output .tmp/native-owned-invocations/app-candidate/libmessenger_mobile.dylib
python3 .tmp/native-owned-invocations/check_app_compatibility.py
node .tmp/native-owned-invocations/check_app_consumer.mjs
```

These app binaries predate the Repo and Ready-order fixes. Their comparison proves legacy binding/descriptor compatibility and observed masks1; it does not replace final-family runtime evidence. The actual app scoped-bindings module is imported as a function, never run through its app-writing CLI. It produces identical175 public/164 scoped operations; the native-only row remains excluded.

Separate initially unapplied SQLite prerequisite research:

```sh
python3 .tmp/native-owned-invocations/core-sqlite-prerequisite/prepare_probe.py .tmp/native-owned-invocations/generated-abi-1794018547245341000/libprogress.dylib
```

This calls embedded SQLite directly to observe prepare-time configuration, not Mesh policy. Its proposal/source/probe pins are separate. The protected Core attempt remains `core-bootstrap/red2/local-result.json`: refusal10 before RPC. No R1 policy, app code,8256 family, native adapter, serializer, service capability or publication path was changed by the research.
