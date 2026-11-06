#!/usr/bin/env bash
# Compile the public API's unsupported-phone boundary; no device execution claim.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"
TOOLCHAIN="${MESH_RUST_TOOLCHAIN:-stable}"
rustc_bin="$(rustup which rustc --toolchain "${TOOLCHAIN}")"
sysroot="$("${rustc_bin}" --print sysroot)"
host="$("${rustc_bin}" -vV | sed -n 's/^host: //p')"
nm="${MESH_LLVM_NM:-${sysroot}/lib/rustlib/${host}/bin/llvm-nm}"
[[ -x "${nm}" ]] || { echo 'matching Rust llvm-nm is required' >&2; exit 1; }
if [[ "$#" -eq 0 ]]; then
  set -- aarch64-apple-ios aarch64-apple-ios-sim aarch64-linux-android x86_64-linux-android
fi
for target in "$@"; do
  build_env=("RUSTC=${rustc_bin}")
  case "${target}" in
    aarch64-apple-ios|aarch64-apple-ios-sim)
      [[ "$(uname -s)" == Darwin ]] || { echo 'iOS compilation requires macOS' >&2; exit 1; }
      ;;
    aarch64-linux-android|x86_64-linux-android)
      ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
      case "$(uname -s)" in
        Darwin) ndk_host=darwin-x86_64 ;;
        Linux) ndk_host=linux-x86_64 ;;
        *) echo 'unsupported NDK host' >&2; exit 1 ;;
      esac
      ndk_bin="${ndk}/toolchains/llvm/prebuilt/${ndk_host}/bin"
      cc="${ndk_bin}/${target}${MESH_ANDROID_API:-24}-clang"
      [[ -x "${cc}" && -x "${ndk_bin}/llvm-ar" ]] || { echo 'set ANDROID_NDK_HOME to an installed NDK' >&2; exit 1; }
      lower="${target//-/_}"
      upper="$(tr '[:lower:]-' '[:upper:]_' <<<"${target}")"
      build_env+=("CC_${lower}=${cc}" "AR_${lower}=${ndk_bin}/llvm-ar" "CARGO_TARGET_${upper}_LINKER=${cc}")
      ;;
    *) echo "unsupported proof target: ${target}" >&2; exit 1 ;;
  esac
  tree="$(rustup run "${TOOLCHAIN}" cargo tree --locked -p mesh-rt --target "${target}" --prefix none)"
  if grep -Eq '^aws-lc-(sys|rs) ' <<<"${tree}"; then
    echo "server RSA provider in ${target} dependency graph" >&2; exit 1
  fi
  env "${build_env[@]}" rustup run "${TOOLCHAIN}" cargo build --locked -p mesh-rt --lib --target "${target}"
  archive="${ROOT}/target/${target}/debug/libmesh_rt.a"
  symbols="$("${nm}" --defined-only --demangle "${archive}")"
  grep -Eq '[[:space:]]_?mesh_storage_key_file$' <<<"${symbols}" || { echo 'missing file API export' >&2; exit 1; }
  if grep -q 'mesh_rt::storage_wrapping::file::' <<<"${symbols}"; then
    echo 'host filesystem backend linked into phone archive' >&2; exit 1
  fi
  python3 - "${target}" "${archive}" <<'PY'
import hashlib, json, pathlib, sys
target, archive = sys.argv[1:]
print('MESH_STORAGE_TARGET_JSON=' + json.dumps({
    'schema_version': 1, 'target': target, 'optimization': 'debug',
    'artifact': archive, 'sha256': hashlib.sha256(pathlib.Path(archive).read_bytes()).hexdigest(),
    'file_api_symbol_present': True, 'host_file_backend_symbols': False,
    'aws_lc_dependency': False, 'compile_only': True, 'executed': False,
}, sort_keys=True))
PY
done
