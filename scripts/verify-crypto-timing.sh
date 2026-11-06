#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT_DIR

usage() {
  printf 'usage: bash scripts/verify-crypto-timing.sh OUTPUT_JSON\n' >&2
}

fail() {
  local message="$1"
  local status="${2:-1}"
  printf 'timing verification failed: %s\n' "${message}" >&2
  exit "${status}"
}

[[ "$#" -eq 1 ]] || {
  usage
  exit 64
}

output_parent="$(dirname "$1")"
output_name="$(basename "$1")"
[[ "${output_name}" != '.' && "${output_name}" != '..' ]] || fail "invalid output path: $1" 73
[[ -d "${output_parent}" ]] || fail "output parent does not exist: ${output_parent}" 73
output_parent="$(cd "${output_parent}" && pwd -P)"
readonly OUTPUT_JSON="${output_parent}/${output_name}"
[[ ! -e "${OUTPUT_JSON}" && ! -L "${OUTPUT_JSON}" ]] || fail "output path already exists: ${OUTPUT_JSON}" 73

for command_name in cargo python3; do
  command -v "${command_name}" >/dev/null || fail "required command is unavailable: ${command_name}" 69
done

log_file="$(mktemp "${TMPDIR:-/tmp}/mesh-crypto-timing.XXXXXX")"
readonly LOG_FILE="${log_file}"
cleanup() {
  [[ -f "${LOG_FILE}" && ! -L "${LOG_FILE}" ]] && rm -- "${LOG_FILE}"
}
trap cleanup EXIT

for timing_test in \
  bytes::tests::secure_equals_timing_distribution \
  crypto::blind_rsa::tests::blind_rsa_sign_timing_distribution; do
  status=0
  (
    cd "${ROOT_DIR}"
    CARGO_INCREMENTAL=0 cargo test --locked --release -p mesh-rt \
      "${timing_test}" -- --ignored --exact --nocapture
  ) >>"${LOG_FILE}" 2>&1 || status=$?
  if [[ "${status}" -ne 0 ]]; then
    tail -n 160 "${LOG_FILE}" >&2
    fail "release timing test ${timing_test} exited ${status}"
  fi
done

python3 - "${LOG_FILE}" "${OUTPUT_JSON}" <<'PY'
import json
import re
import sys

log_path, output_path = sys.argv[1:]
with open(log_path, encoding="utf-8") as log_file:
    matches = re.findall(r"MESH_TIMING_JSON=(\{[^\n]+\})", log_file.read())

expected = {"Bytes.secure_equals", "Crypto.blind_rsa_sign"}
records = [json.loads(match) for match in matches]
if len(records) != 2 or {r.get("boundary") for r in records} != expected:
    raise SystemExit("expected exactly one timing record for secure_equals and blind_rsa_sign")
for record in records:
    if record.get("schema_version") != 2:
        raise SystemExit("timing record has an unexpected schema")
    if record.get("samples_per_group", 0) < 200 or record.get("passed") is not True:
        raise SystemExit("timing record did not satisfy the release contract")
    if record.get("inconclusive") is True:
        print(
            f"warning: {record['boundary']} timing was INCONCLUSIVE on this host "
            f"(control |t|={record.get('control_t')}, measured |t|={record.get('welch_t')}); "
            "re-run on a quiet machine before treating it as evidence",
            file=sys.stderr,
        )
record = {
    "schema_version": 3,
    "boundaries": records,
    "passed": True,
    "inconclusive": any(r.get("inconclusive") is True for r in records),
}

with open(output_path, "x", encoding="utf-8") as output_file:
    json.dump(record, output_file, indent=2, sort_keys=True)
    output_file.write("\n")
PY

printf 'timing evidence recorded: %s\n' "${OUTPUT_JSON}"
