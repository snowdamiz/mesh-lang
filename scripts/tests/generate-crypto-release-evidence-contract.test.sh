#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT_DIR
readonly SCRIPT_PATH="${ROOT_DIR}/scripts/generate-crypto-release-evidence.sh"

fail() {
  printf 'contract failure: %s\n' "$*" >&2
  exit 1
}

status=0
output="$(bash "${SCRIPT_PATH}" 2>&1)" || status=$?
[[ "${status}" -eq 64 ]] || fail "missing output path exited ${status}, expected 64"
grep -Fq 'usage:' <<<"${output}" || fail "missing output path did not print usage"

existing_dir="$(mktemp -d "${TMPDIR:-/tmp}/mesh-release-evidence-contract.XXXXXX")"
[[ -d "${existing_dir}" && ! -L "${existing_dir}" ]] || fail "mktemp did not create a safe directory"
trap 'rmdir -- "${existing_dir}"' EXIT

status=0
output="$(bash "${SCRIPT_PATH}" "${existing_dir}" 2>&1)" || status=$?
[[ "${status}" -eq 73 ]] || fail "existing output path exited ${status}, expected 73"
grep -Fq 'output path already exists' <<<"${output}" || fail "existing output path was not rejected"

printf 'release evidence contract passed\n'
