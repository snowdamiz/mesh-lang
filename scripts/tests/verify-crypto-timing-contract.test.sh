#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT_DIR
readonly SCRIPT_PATH="${ROOT_DIR}/scripts/verify-crypto-timing.sh"

fail() {
  printf 'contract failure: %s\n' "$*" >&2
  exit 1
}

status=0
output="$(bash "${SCRIPT_PATH}" 2>&1)" || status=$?
[[ "${status}" -eq 64 ]] || fail "missing output path exited ${status}, expected 64"
grep -Fq 'usage:' <<<"${output}" || fail "missing output path did not print usage"

existing_file="$(mktemp "${TMPDIR:-/tmp}/mesh-timing-contract.XXXXXX")"
[[ -f "${existing_file}" && ! -L "${existing_file}" ]] || fail "mktemp did not create a safe file"
trap 'rm -- "${existing_file}"' EXIT

status=0
output="$(bash "${SCRIPT_PATH}" "${existing_file}" 2>&1)" || status=$?
[[ "${status}" -eq 73 ]] || fail "existing output path exited ${status}, expected 73"
grep -Fq 'output path already exists' <<<"${output}" || fail "existing output path was not rejected"

printf 'timing harness contract passed\n'
