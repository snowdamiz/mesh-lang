#!/usr/bin/env bash
set -euo pipefail

readonly MOBILE_TARGET="aarch64-apple-ios"
readonly RUST_TOOLCHAIN="${MESH_RUST_TOOLCHAIN:-stable}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
REPOSITORY_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly REPOSITORY_ROOT
readonly MOBILE_STATICLIB="${REPOSITORY_ROOT}/target/${MOBILE_TARGET}/debug/libmesh_rt.a"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

[[ "$(uname -s)" == "Darwin" ]] || fail "${MOBILE_TARGET} verification requires macOS"
command -v rustup >/dev/null || fail "rustup is required"
command -v xcrun >/dev/null || fail "Xcode command-line tools are required"
xcrun --sdk iphoneos --show-sdk-path >/dev/null

if ! rustup target list --installed --toolchain "${RUST_TOOLCHAIN}" | grep -Fxq "${MOBILE_TARGET}"; then
  fail "install the target with: rustup target add ${MOBILE_TARGET} --toolchain ${RUST_TOOLCHAIN}"
fi

RUSTC_BIN="$(rustup which rustc --toolchain "${RUST_TOOLCHAIN}")"
readonly RUSTC_BIN
[[ -x "${RUSTC_BIN}" ]] || fail "rustc is unavailable for toolchain ${RUST_TOOLCHAIN}"

cd "${REPOSITORY_ROOT}"
RUSTC="${RUSTC_BIN}" rustup run "${RUST_TOOLCHAIN}" cargo build \
  --locked \
  -p mesh-rt \
  --lib \
  --target "${MOBILE_TARGET}" \
  --target-dir "${REPOSITORY_ROOT}/target"

[[ -s "${MOBILE_STATICLIB}" ]] || fail "missing iOS static library: ${MOBILE_STATICLIB}"
printf 'verified: %s\n' "${MOBILE_STATICLIB}"
