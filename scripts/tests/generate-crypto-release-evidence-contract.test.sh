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

grep -Fq 'tests/vectors/mlkem/mlkem768-keygen-acvp-tc26.json' "${SCRIPT_PATH}" ||
  fail "release evidence does not publish the ML-KEM vector"
grep -Fq 'crypto_v2_public_api_compiles_and_executes_natively' "${SCRIPT_PATH}" ||
  fail "release evidence does not run the public Mesh vector proof"
grep -Fq 'known-answer-vectors.log' "${SCRIPT_PATH}" ||
  fail "release evidence does not retain the vector runner report"
grep -Fq 'known_answer_vectors' "${SCRIPT_PATH}" ||
  fail "release record does not name the vector result"
grep -Fq 'tests/vectors/blind-rsa/rfc9578-type2.json' "${SCRIPT_PATH}" ||
  fail "release evidence does not publish the RFC 9578 blind RSA vectors"
for proof in blind_rsa_rfc9578_type2_vectors_run_through_the_public_mesh_api \
  blind_rsa_agrees_with_the_openssl_cli; do
  grep -Fq "${proof}" "${SCRIPT_PATH}" ||
    fail "release evidence does not run ${proof}"
done
grep -Fq 'crypto::mlkem_tests' "${SCRIPT_PATH}" ||
  fail "release evidence does not run the ACVP and OpenSSL ML-KEM vectors"
grep -Fq 'blind_rsa_sign_timing' "${SCRIPT_PATH}" ||
  fail "release record does not name the blind RSA signing timing result"
grep -Fq 'cargo audit --json >"${OUTPUT_DIR}/cargo-audit.json"' "${SCRIPT_PATH}" ||
  fail "release evidence does not audit the actual workspace lockfile"
grep -Fq 'cargo audit --deny warnings --json --file scripts/openraft-upstream-audit.lock' "${SCRIPT_PATH}" ||
  fail "release evidence does not retain advisory coverage of vendored OpenRaft"
grep -Fq 'openraft_upstream_audit' "${SCRIPT_PATH}" ||
  fail "release record does not name the upstream OpenRaft audit result"
python3 "${ROOT_DIR}/scripts/verify-openraft-audit-coverage.py"
python3 "${ROOT_DIR}/scripts/tests/verify-openraft-audit-coverage.test.py"
grep -Fq 'cargo audit --deny warnings --json --file scripts/ureq-upstream-audit.lock' "${SCRIPT_PATH}" ||
  fail "release evidence does not retain advisory coverage of vendored ureq"
grep -Fq 'ureq_upstream_audit' "${SCRIPT_PATH}" ||
  fail "release record does not name the upstream ureq audit result"
python3 "${ROOT_DIR}/scripts/verify-ureq-audit-coverage.py"

printf 'release evidence contract passed\n'
