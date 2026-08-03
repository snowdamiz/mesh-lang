#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT_DIR

usage() {
  printf 'usage: bash scripts/fuzz-smoke.sh OUTPUT_DIRECTORY [SECONDS_PER_TARGET]\n' >&2
}

fail() {
  local message="$1"
  local status="${2:-1}"
  printf 'fuzz smoke failed: %s\n' "${message}" >&2
  exit "${status}"
}

[[ "$#" -ge 1 && "$#" -le 2 ]] || {
  usage
  exit 64
}

seconds="${2:-60}"
[[ "${seconds}" =~ ^[0-9]+$ && "${seconds}" -ge 1 && "${seconds}" -le 3600 ]] || \
  fail "seconds per target must be an integer from 1 through 3600" 64

output_parent="$(dirname "$1")"
output_name="$(basename "$1")"
[[ "${output_name}" != '.' && "${output_name}" != '..' ]] || fail "invalid output path: $1" 73
[[ -d "${output_parent}" ]] || fail "output parent does not exist: ${output_parent}" 73
output_parent="$(cd "${output_parent}" && pwd -P)"
readonly OUTPUT_DIR="${output_parent}/${output_name}"
[[ ! -e "${OUTPUT_DIR}" && ! -L "${OUTPUT_DIR}" ]] || fail "output path already exists: ${OUTPUT_DIR}" 73

for command_name in git python3 rustup; do
  command -v "${command_name}" >/dev/null || fail "required command is unavailable: ${command_name}" 69
done
nightly_cargo="$(rustup which --toolchain nightly cargo 2>/dev/null)" || \
  fail "nightly Rust is unavailable" 69
readonly nightly_cargo
[[ -x "${nightly_cargo}" ]] || fail "nightly Cargo is unavailable" 69
PATH="$(dirname "${nightly_cargo}"):${PATH}"
export PATH
cargo fuzz --version >/dev/null 2>&1 || fail "cargo-fuzz is unavailable for nightly Rust" 69

mkdir "${OUTPUT_DIR}"
chmod 700 "${OUTPUT_DIR}"
readonly TARGETS=(byte_operations crypto_provider runtime_decoders source_parser)

cd "${ROOT_DIR}"
for target in "${TARGETS[@]}"; do
  cargo fuzz run "${target}" -- \
    "-max_total_time=${seconds}" \
    -max_len=65536 \
    -timeout=10 \
    -rss_limit_mb=4096 \
    -print_final_stats=1 \
    >"${OUTPUT_DIR}/${target}.log" 2>&1 || {
      sed -n '1,240p' "${OUTPUT_DIR}/${target}.log" >&2
      fail "target failed: ${target}"
    }
done

revision="$(git rev-parse --verify HEAD)"
python3 - "${OUTPUT_DIR}/fuzz-smoke.json" "${revision}" "${seconds}" "${TARGETS[@]}" <<'PY'
from datetime import datetime, timezone
import json
import sys

path, revision, seconds, *targets = sys.argv[1:]
record = {
    "schema_version": 1,
    "generated_at": datetime.now(timezone.utc).isoformat(),
    "revision": revision,
    "seconds_per_target": int(seconds),
    "targets": [{"name": target, "status": "passed"} for target in targets],
}
with open(path, "x", encoding="utf-8") as output:
    json.dump(record, output, indent=2, sort_keys=True)
    output.write("\n")
PY

printf 'fuzz smoke passed: %s\n' "${OUTPUT_DIR}"
