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
scratch="$(mktemp -d "${TMPDIR:-/tmp}/mesh-timing-contract-dir.XXXXXX")"
trap 'rm -- "${existing_file}"; rm -rf -- "${scratch:?}"' EXIT

status=0
output="$(bash "${SCRIPT_PATH}" "${existing_file}" 2>&1)" || status=$?
[[ "${status}" -eq 73 ]] || fail "existing output path exited ${status}, expected 73"
grep -Fq 'output path already exists' <<<"${output}" || fail "existing output path was not rejected"

# A successful cargo process without the BR1 record is not timing evidence.
mkdir "${scratch}/bin"
cat >"${scratch}/bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' 'MESH_TIMING_JSON={"schema_version":2,"boundary":"Bytes.secure_equals","samples_per_group":200,"passed":true,"inconclusive":false}'
STUB
chmod +x "${scratch}/bin/cargo"
status=0
output="$(PATH="${scratch}/bin:${PATH}" bash "${SCRIPT_PATH}" "${scratch}/missing-br1.json" 2>&1)" || status=$?
[[ "${status}" -ne 0 ]] || fail "missing BR1 timing evidence was accepted"

# The complete pair is accepted and remains a structured two-boundary record.
cat >"${scratch}/bin/cargo" <<'STUB'
#!/usr/bin/env bash
boundary='Bytes.secure_equals'
if [[ "$*" == *blind_rsa_sign_timing_distribution* ]]; then boundary='Crypto.blind_rsa_sign'; fi
printf 'MESH_TIMING_JSON={"schema_version":2,"boundary":"%s","samples_per_group":200,"passed":true,"inconclusive":false}\n' "${boundary}"
STUB
PATH="${scratch}/bin:${PATH}" bash "${SCRIPT_PATH}" "${scratch}/complete.json" >/dev/null
python3 - "${scratch}/complete.json" <<'PY'
import json
import sys
with open(sys.argv[1]) as source:
    record = json.load(source)
assert record["schema_version"] == 3
assert {b["boundary"] for b in record["boundaries"]} == {"Bytes.secure_equals", "Crypto.blind_rsa_sign"}
assert record["passed"] and record["inconclusive"] is False
PY

printf 'timing harness contract passed\n'
