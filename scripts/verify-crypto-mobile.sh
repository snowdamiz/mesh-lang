#!/usr/bin/env bash
set -euo pipefail

readonly RUST_TOOLCHAIN="${MESH_RUST_TOOLCHAIN:-stable}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
REPOSITORY_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly REPOSITORY_ROOT

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

# No target argument means the complete advertised phone compilation matrix.
# An explicit subset is useful locally and is labelled as such in each record.
if [[ "$#" -eq 0 ]]; then
  set -- aarch64-apple-ios aarch64-linux-android
fi
for target in "$@"; do
  case "${target}" in
    aarch64-apple-ios|aarch64-linux-android) ;;
    *) fail "unsupported proof target: ${target}" ;;
  esac
done
command -v rustup >/dev/null || fail "rustup is required"
command -v python3 >/dev/null || fail "python3 is required"
RUSTC_BIN="$(rustup which rustc --toolchain "${RUST_TOOLCHAIN}")"
readonly RUSTC_BIN
[[ -x "${RUSTC_BIN}" ]] || fail "rustc is unavailable for toolchain ${RUST_TOOLCHAIN}"
RUST_SYSROOT="$("${RUSTC_BIN}" --print sysroot)"
RUST_HOST="$("${RUSTC_BIN}" -vV | sed -n 's/^host: //p')"
LLVM_NM="${MESH_LLVM_NM:-${RUST_SYSROOT}/lib/rustlib/${RUST_HOST}/bin/llvm-nm}"
if ! command -v "${LLVM_NM}" >/dev/null && [[ -z "${MESH_LLVM_NM:-}" ]]; then
  LLVM_NM=llvm-nm
  if ! command -v "${LLVM_NM}" >/dev/null && command -v xcrun >/dev/null; then
    LLVM_NM="$(xcrun -f llvm-nm)"
  fi
fi
command -v "${LLVM_NM}" >/dev/null || fail "install matching tools: rustup component add llvm-tools-preview --toolchain ${RUST_TOOLCHAIN} (or set MESH_LLVM_NM)"
readonly LLVM_NM

cd "${REPOSITORY_ROOT}"
for target in "$@"; do
  if [[ "${target}" == aarch64-apple-ios ]]; then
    [[ "$(uname -s)" == Darwin ]] || fail "${target} verification requires macOS"
    command -v xcrun >/dev/null || fail "Xcode command-line tools are required"
    xcrun --sdk iphoneos --show-sdk-path >/dev/null
  fi
  if ! rustup target list --installed --toolchain "${RUST_TOOLCHAIN}" | grep -Fxq "${target}"; then
    fail "install the target with: rustup target add ${target} --toolchain ${RUST_TOOLCHAIN}"
  fi
  # A target-specific dependency tree catches accidental server-provider
  # activation even if link-time dead-code elimination hides the symbols.
  dependencies="$(rustup run "${RUST_TOOLCHAIN}" cargo tree --locked -p mesh-rt --target "${target}" --prefix none)"
  if grep -Eq '^aws-lc-(sys|rs) ' <<<"${dependencies}"; then
    fail "AWS-LC appears in the ${target} dependency graph"
  fi
  build_env=("RUSTC=${RUSTC_BIN}")
  if [[ "${target}" == aarch64-linux-android ]]; then
    ndk_root="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
    [[ -n "${ndk_root}" && -d "${ndk_root}" ]] || fail "set ANDROID_NDK_HOME to an installed Android NDK"
    case "$(uname -s)" in
      Darwin) ndk_host=darwin-x86_64 ;;
      Linux) ndk_host=linux-x86_64 ;;
      *) fail "Android compilation needs a supported NDK host" ;;
    esac
    ndk_bin="${ndk_root}/toolchains/llvm/prebuilt/${ndk_host}/bin"
    android_cc="${ndk_bin}/aarch64-linux-android${MESH_ANDROID_API:-24}-clang"
    [[ -x "${android_cc}" && -x "${ndk_bin}/llvm-ar" ]] || fail "Android NDK compiler/ar unavailable: ${ndk_bin}"
    build_env+=("CC_aarch64_linux_android=${android_cc}" "AR_aarch64_linux_android=${ndk_bin}/llvm-ar" "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=${android_cc}")
  fi
  env "${build_env[@]}" rustup run "${RUST_TOOLCHAIN}" cargo build \
    --locked --release -p mesh-rt --lib --target "${target}" \
    --target-dir "${REPOSITORY_ROOT}/target"
  staticlib="${REPOSITORY_ROOT}/target/${target}/release/libmesh_rt.a"
  [[ -s "${staticlib}" ]] || fail "missing static library: ${staticlib}"
  symbols="$("${LLVM_NM}" --defined-only --extern-only "${staticlib}")" ||
    fail "symbol inspection failed; LLVM tools must match rustc (MESH_LLVM_NM)"
  for client_symbol in \
    mesh_crypto_blind_rsa_public_from_spki \
    mesh_crypto_blind_rsa_blind \
    mesh_crypto_blind_rsa_finalize \
    mesh_crypto_blind_rsa_verify \
    mesh_blind_rsa_blinding_state_seal_for_storage \
    mesh_blind_rsa_blinding_state_unseal_from_storage; do
    grep -Eq "[[:space:]]_?${client_symbol}$" <<<"${symbols}" || fail "${target} omits ${client_symbol}"
  done
  if grep -Eq '[[:space:]]_?([[:alnum:]_]*aws_lc[[:alnum:]_]*_)?RSA_(sign_raw|generate_key_ex|private_transform)$' <<<"${symbols}"; then
    fail "AWS-LC private RSA implementation linked into ${target}"
  fi
  python3 - "${target}" "${staticlib}" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

target, artifact = sys.argv[1:]
print("MESH_CRYPTO_TARGET_JSON=" + json.dumps({
    "schema_version": 1,
    "profile": "BR1",
    "target": target,
    "optimization": "release",
    "artifact": artifact,
    "sha256": hashlib.sha256(Path(artifact).read_bytes()).hexdigest(),
    "aws_lc_dependency": False,
    "aws_lc_private_symbols": False,
    "client_symbols_present": True,
    "compile_only": True,
    "vectors_executed": False,
}, sort_keys=True))
PY
  printf 'verified compilation/provider exclusion: %s\n' "${staticlib}"
done
