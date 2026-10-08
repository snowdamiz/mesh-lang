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

status=0
(
  cd "${ROOT_DIR}"
  # Bytes.secure_equals, and blind RSA signing on AWS-LC (servers only, as
  # the signer is: Linux and macOS).
  CARGO_INCREMENTAL=0 cargo test --locked --release -p mesh-rt --lib -- \
    bytes::tests::secure_equals_timing_distribution \
    crypto::blind_rsa::tests::server::blind_rsa_sign_timing_distribution \
    --ignored --exact --nocapture --test-threads=1
) >"${LOG_FILE}" 2>&1 || status=$?

if [[ "${status}" -ne 0 ]]; then
  sed -n '1,240p' "${LOG_FILE}" >&2
  fail "release timing test exited ${status}"
fi

python3 - "${LOG_FILE}" "${OUTPUT_JSON}" <<'PY'
import json
import re
import sys

log_path, output_path = sys.argv[1:]
with open(log_path, encoding="utf-8") as log_file:
    matches = re.findall(r"MESH_TIMING_JSON=(\{[^\n]+\})", log_file.read())

BOUNDARIES = ["Bytes.secure_equals", "Crypto.blind_rsa_sign"]
records = [json.loads(match) for match in matches]
if sorted(record.get("boundary") for record in records) != sorted(BOUNDARIES):
    raise SystemExit(
        f"expected one timing record for each of {BOUNDARIES}, found {len(records)}"
    )
for record in records:
    if record.get("schema_version") != 2:
        raise SystemExit("timing record has an unexpected schema")
    if record.get("samples_per_group", 0) < 200 or record.get("passed") is not True:
        raise SystemExit(f"{record['boundary']} timing did not satisfy the release contract")
    if record.get("inconclusive") is True:
        # Not a leak and not a clean bill of health: the control group -- an
        # identical workload in its own allocation -- separated by as much as
        # the real comparison, so this host cannot resolve the boundary at
        # all. Say so in the evidence rather than recording a pass that was
        # never measured.
        print(
            f"warning: {record['boundary']} timing was INCONCLUSIVE on this host "
            f"(control |t|={record.get('control_t')}, "
            f"compared |t|={record.get('welch_t')}); "
            "re-run on a quiet machine before treating it as evidence",
            file=sys.stderr,
        )

with open(output_path, "x", encoding="utf-8") as output_file:
    json.dump(
        {
            "schema_version": 3,
            "boundaries": {record["boundary"]: record for record in records},
        },
        output_file,
        indent=2,
        sort_keys=True,
    )
    output_file.write("\n")
PY

printf 'timing verification passed: %s\n' "${OUTPUT_JSON}"
