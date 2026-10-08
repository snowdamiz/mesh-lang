#!/usr/bin/env bash
set -euo pipefail

# Build the runtime static library for the phone targets and prove the
# server-only blind RSA provider is not in it: AWS-LC is absent from each
# target's dependency graph and from the archive, while the blind RSA entry
# points are present (the server-only ones return UnsupportedTarget there).
# ML-KEM must come from libcrux-ml-kem 0.0.10, never the ml-kem crate.
# CARGO_TARGET_DIR selects the build directory (default: target/).
#
# usage: bash scripts/verify-crypto-mobile.sh [TARGET...]
#
# Targets default to aarch64-apple-ios and aarch64-linux-android.
# aarch64-apple-ios needs macOS with Xcode. aarch64-linux-android needs the
# Android NDK: ANDROID_NDK_HOME, or the newest ndk/<version> under
# ANDROID_HOME, ANDROID_SDK_ROOT, ~/Library/Android/sdk or the Homebrew
# android-commandlinetools cask.

readonly RUST_TOOLCHAIN="${MESH_RUST_TOOLCHAIN:-stable}"
readonly ANDROID_API="${MESH_ANDROID_API:-24}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
REPOSITORY_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly REPOSITORY_ROOT
readonly TARGET_DIR="${CARGO_TARGET_DIR:-${REPOSITORY_ROOT}/target}"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

command -v rustup >/dev/null || fail "rustup is required"
command -v nm >/dev/null || fail "nm is required"
RUSTC_BIN="$(rustup which rustc --toolchain "${RUST_TOOLCHAIN}")"
readonly RUSTC_BIN
[[ -x "${RUSTC_BIN}" ]] || fail "rustc is unavailable for toolchain ${RUST_TOOLCHAIN}"

android_ndk() {
  if [[ -n "${ANDROID_NDK_HOME:-}" ]]; then
    printf '%s\n' "${ANDROID_NDK_HOME}"
    return
  fi
  local sdk newest
  for sdk in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "${HOME}/Library/Android/sdk" \
    /opt/homebrew/share/android-commandlinetools /usr/local/share/android-commandlinetools; do
    [[ -n "${sdk}" && -d "${sdk}/ndk" ]] || continue
    newest="$(find "${sdk}/ndk" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -n 1)"
    if [[ -n "${newest}" ]]; then
      printf '%s\n' "${newest}"
      return
    fi
  done
  fail "aarch64-linux-android needs the Android NDK: set ANDROID_NDK_HOME"
}

prepare_target() {
  local target="$1"
  case "${target}" in
    *-apple-ios*)
      [[ "$(uname -s)" == "Darwin" ]] || fail "${target} verification requires macOS"
      command -v xcrun >/dev/null || fail "Xcode command-line tools are required for ${target}"
      xcrun --sdk iphoneos --show-sdk-path >/dev/null
      ;;
    *-linux-android*)
      local ndk toolchain variable upper
      ndk="$(android_ndk)"
      toolchain="$(find "${ndk}/toolchains/llvm/prebuilt" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
      [[ -x "${toolchain}/bin/clang" ]] || fail "no NDK clang under ${ndk}"
      variable="${target//-/_}"
      upper="$(tr '[:lower:]' '[:upper:]' <<<"${variable}")"
      export "CC_${variable}=${toolchain}/bin/${target}${ANDROID_API}-clang"
      export "AR_${variable}=${toolchain}/bin/llvm-ar"
      export "CARGO_TARGET_${upper}_LINKER=${toolchain}/bin/${target}${ANDROID_API}-clang"
      ;;
    *)
      fail "unsupported mobile target: ${target}"
      ;;
  esac
}

verify_target() {
  local target="$1"
  if ! rustup target list --installed --toolchain "${RUST_TOOLCHAIN}" | grep -Fxq "${target}"; then
    fail "install the target with: rustup target add ${target} --toolchain ${RUST_TOOLCHAIN}"
  fi
  prepare_target "${target}"

  local graph
  graph="$(cd "${REPOSITORY_ROOT}" && RUSTC="${RUSTC_BIN}" rustup run "${RUST_TOOLCHAIN}" \
    cargo tree --locked -p mesh-rt --target "${target}" -e normal,build --prefix none)"
  ! grep -q '^aws-lc-sys ' <<<"${graph}" ||
    fail "${target}: aws-lc-sys is in the mesh-rt dependency graph"
  grep -q '^libcrux-ml-kem v0\.0\.10$' <<<"${graph}" ||
    fail "${target}: ML-KEM is not provided by libcrux-ml-kem 0.0.10"
  ! grep -q '^ml-kem ' <<<"${graph}" ||
    fail "${target}: the ml-kem crate is in the mesh-rt dependency graph"

  (
    cd "${REPOSITORY_ROOT}"
    RUSTC="${RUSTC_BIN}" rustup run "${RUST_TOOLCHAIN}" cargo build \
      --locked \
      -p mesh-rt \
      --lib \
      --target "${target}" \
      --target-dir "${TARGET_DIR}"
  )

  local staticlib="${TARGET_DIR}/${target}/debug/libmesh_rt.a"
  [[ -s "${staticlib}" ]] || fail "missing ${target} static library: ${staticlib}"
  local symbols entry
  symbols="$(nm -g "${staticlib}" 2>/dev/null || true)"
  ! grep -q 'aws_lc_' <<<"${symbols}" ||
    fail "${target}: AWS-LC symbols are linked into ${staticlib}"
  for entry in mesh_crypto_blind_rsa_blind mesh_crypto_blind_rsa_finalize \
    mesh_crypto_blind_rsa_verify mesh_crypto_blind_rsa_sign; do
    grep -Eq " T _?${entry}$" <<<"${symbols}" || fail "${target}: ${entry} is missing"
  done
  printf 'verified: %s (no AWS-LC; libcrux ML-KEM; blind RSA entry points present)\n' "${staticlib}"
}

targets=("$@")
[[ "${#targets[@]}" -gt 0 ]] || targets=(aarch64-apple-ios aarch64-linux-android)
for target in "${targets[@]}"; do
  verify_target "${target}"
done
