#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT_DIR
readonly PROFILE="crypto-v2-development-baseline"

usage() {
  printf 'usage: bash scripts/generate-crypto-release-evidence.sh OUTPUT_DIRECTORY\n' >&2
}

fail() {
  local message="$1"
  local status="${2:-1}"
  printf 'release evidence failed: %s\n' "${message}" >&2
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
readonly OUTPUT_DIR="${output_parent}/${output_name}"
[[ ! -e "${OUTPUT_DIR}" && ! -L "${OUTPUT_DIR}" ]] || fail "output path already exists: ${OUTPUT_DIR}" 73

for command_name in cargo cargo-audit cargo-cyclonedx git python3 rustc tar; do
  command -v "${command_name}" >/dev/null || fail "required command is unavailable: ${command_name}" 69
done

cd "${ROOT_DIR}"
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || fail "release evidence requires a clean tracked worktree" 65

REVISION="$(git rev-parse --verify HEAD)"
readonly REVISION
COMMIT_EPOCH="$(git show -s --format=%ct "${REVISION}")"
readonly COMMIT_EPOCH
RUSTC_VERSION="$(rustc --version)"
readonly RUSTC_VERSION
CARGO_VERSION="$(cargo --version)"
readonly CARGO_VERSION
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
readonly HOST_TARGET
AUDIT_VERSION="$(cargo audit --version)"
readonly AUDIT_VERSION
CYCLONEDX_VERSION="$(cargo cyclonedx --version)"
readonly CYCLONEDX_VERSION

mkdir "${OUTPUT_DIR}"
chmod 700 "${OUTPUT_DIR}"

scratch_root="$(mktemp -d "${TMPDIR:-/tmp}/mesh-crypto-release-evidence.XXXXXX")"
SCRATCH_ROOT="$(cd "${scratch_root}" && pwd -P)"
readonly SCRATCH_ROOT
TEMP_PARENT="$(cd "${TMPDIR:-/tmp}" && pwd -P)"
readonly TEMP_PARENT
[[ -d "${SCRATCH_ROOT}" && ! -L "${SCRATCH_ROOT}" && "${SCRATCH_ROOT}" == "${TEMP_PARENT}/mesh-crypto-release-evidence."* ]] || fail "unsafe scratch path: ${SCRATCH_ROOT}"

cleanup() {
  [[ -n "${SCRATCH_ROOT:-}" && -d "${SCRATCH_ROOT}" && ! -L "${SCRATCH_ROOT}" ]] || return
  [[ "${SCRATCH_ROOT}" == "${TEMP_PARENT}/mesh-crypto-release-evidence."* ]] || return
  rm -rf -- "${SCRATCH_ROOT:?}"
}
trap cleanup EXIT

readonly SOURCE_A="${SCRATCH_ROOT}/source-a"
readonly SOURCE_B="${SCRATCH_ROOT}/source-b"
readonly BUILD_A="${SCRATCH_ROOT}/build-a"
readonly BUILD_B="${SCRATCH_ROOT}/build-b"
readonly TIMING_BUILD="${SCRATCH_ROOT}/timing-build"

mkdir "${SOURCE_A}" "${SOURCE_B}"
git archive "${REVISION}" | tar -x -C "${SOURCE_A}"
git archive "${REVISION}" | tar -x -C "${SOURCE_B}"

audit_status=0
(cd "${SOURCE_A}" && cargo audit --json >"${OUTPUT_DIR}/cargo-audit.json") || audit_status=$?
[[ "${audit_status}" -eq 0 ]] || fail "cargo audit reported findings; see ${OUTPUT_DIR}/cargo-audit.json"

python3 - "${OUTPUT_DIR}/cargo-audit.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as audit_file:
    audit = json.load(audit_file)

vulnerabilities = audit.get("vulnerabilities", {}).get("count")
warnings = sum(len(entries) for entries in audit.get("warnings", {}).values())
if vulnerabilities != 0 or warnings != 0:
    raise SystemExit(f"audit is not clean: vulnerabilities={vulnerabilities}, warnings={warnings}")
PY

(
  cd "${SOURCE_A}"
  cargo cyclonedx \
    --manifest-path compiler/meshc/Cargo.toml \
    --format json \
    --describe binaries \
    --target all \
    --license-strict \
    --license-accept-named 'MIT/Apache-2.0' \
    --license-accept-named 'Apache-2.0/MIT' \
    --spec-version 1.5
) >"${OUTPUT_DIR}/cyclonedx.log" 2>&1

readonly GENERATED_SBOM="${SOURCE_A}/compiler/meshc/meshc_bin.cdx.json"
[[ -s "${GENERATED_SBOM}" ]] || fail "cargo cyclonedx did not produce the meshc SBOM"
mv "${GENERATED_SBOM}" "${OUTPUT_DIR}/meshc.cdx.json"

python3 - "${OUTPUT_DIR}/meshc.cdx.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as sbom_file:
    sbom = json.load(sbom_file)

component = sbom.get("metadata", {}).get("component", {})
if sbom.get("bomFormat") != "CycloneDX" or sbom.get("specVersion") != "1.5":
    raise SystemExit("SBOM is not CycloneDX 1.5 JSON")
if component.get("name") != "meshc" or not sbom.get("components"):
    raise SystemExit("SBOM does not describe meshc and its resolved components")
PY

(
  cd "${SOURCE_A}"
  CARGO_TARGET_DIR="${TIMING_BUILD}" \
    bash scripts/verify-crypto-timing.sh "${OUTPUT_DIR}/constant-time.json"
) >"${OUTPUT_DIR}/constant-time.log" 2>&1
cargo clean --manifest-path "${SOURCE_A}/Cargo.toml" --target-dir "${TIMING_BUILD}" >/dev/null

sha256_file() {
  python3 - "$1" <<'PY'
from hashlib import sha256
from pathlib import Path
import sys

print(sha256(Path(sys.argv[1]).read_bytes()).hexdigest())
PY
}

build_once() {
  local source_root="$1"
  local target_root="$2"
  local log_path="$3"
  local remap_flags="--remap-path-prefix=${source_root}=/mesh-src"
  if [[ -n "${RUSTFLAGS:-}" ]]; then
    remap_flags="${RUSTFLAGS} ${remap_flags}"
  fi

  (
    cd "${source_root}"
    CARGO_INCREMENTAL=0 \
      SOURCE_DATE_EPOCH="${COMMIT_EPOCH}" \
      RUSTFLAGS="${remap_flags}" \
      cargo build --locked --release -p meshc --target-dir "${target_root}"
  ) >"${log_path}" 2>&1
}

binary_name="meshc"
[[ "${HOST_TARGET}" != *-windows-* ]] || binary_name="meshc.exe"

build_once "${SOURCE_A}" "${BUILD_A}" "${OUTPUT_DIR}/build-a.log"
readonly BINARY_A="${BUILD_A}/release/${binary_name}"
[[ -s "${BINARY_A}" ]] || fail "first release build did not produce ${binary_name}"
SHA_A="$(sha256_file "${BINARY_A}")"
readonly SHA_A
SIZE_A="$(wc -c <"${BINARY_A}" | tr -d ' ')"
readonly SIZE_A
cargo clean --manifest-path "${SOURCE_A}/Cargo.toml" --target-dir "${BUILD_A}" >/dev/null

build_once "${SOURCE_B}" "${BUILD_B}" "${OUTPUT_DIR}/build-b.log"
readonly BINARY_B="${BUILD_B}/release/${binary_name}"
[[ -s "${BINARY_B}" ]] || fail "second release build did not produce ${binary_name}"
SHA_B="$(sha256_file "${BINARY_B}")"
readonly SHA_B
SIZE_B="$(wc -c <"${BINARY_B}" | tr -d ' ')"
readonly SIZE_B

reproducible=false
[[ "${SHA_A}" == "${SHA_B}" && "${SIZE_A}" == "${SIZE_B}" ]] && reproducible=true

python3 - "${OUTPUT_DIR}/reproducibility.json" "${REVISION}" "${HOST_TARGET}" "${COMMIT_EPOCH}" "${SHA_A}" "${SHA_B}" "${SIZE_A}" "${SIZE_B}" "${reproducible}" <<'PY'
import json
import sys

path, revision, target, epoch, sha_a, sha_b, size_a, size_b, reproducible = sys.argv[1:]
with open(path, "w", encoding="utf-8") as output:
    json.dump(
        {
            "schema_version": 1,
            "revision": revision,
            "target": target,
            "source_date_epoch": int(epoch),
            "artifact": "meshc",
            "build_a": {"sha256": sha_a, "size": int(size_a)},
            "build_b": {"sha256": sha_b, "size": int(size_b)},
            "reproducible": reproducible == "true",
        },
        output,
        indent=2,
        sort_keys=True,
    )
    output.write("\n")
PY

cargo clean --manifest-path "${SOURCE_B}/Cargo.toml" --target-dir "${BUILD_B}" >/dev/null
[[ "${reproducible}" == true ]] || fail "isolated meshc release builds differ; see ${OUTPUT_DIR}/reproducibility.json"

AUDIT_SHA="$(sha256_file "${OUTPUT_DIR}/cargo-audit.json")"
readonly AUDIT_SHA
SBOM_SHA="$(sha256_file "${OUTPUT_DIR}/meshc.cdx.json")"
readonly SBOM_SHA
TIMING_SHA="$(sha256_file "${OUTPUT_DIR}/constant-time.json")"
readonly TIMING_SHA

python3 - "${OUTPUT_DIR}/release-record.json" "${PROFILE}" "${REVISION}" "${COMMIT_EPOCH}" "${HOST_TARGET}" "${RUSTC_VERSION}" "${CARGO_VERSION}" "${AUDIT_VERSION}" "${CYCLONEDX_VERSION}" "${AUDIT_SHA}" "${SBOM_SHA}" "${TIMING_SHA}" "${SHA_A}" <<'PY'
from datetime import datetime, timezone
import json
import sys

(
    path,
    profile,
    revision,
    epoch,
    target,
    rustc,
    cargo,
    audit_tool,
    cyclonedx_tool,
    audit_sha,
    sbom_sha,
    timing_sha,
    artifact_sha,
) = sys.argv[1:]

record = {
    "schema_version": 1,
    "generated_at": datetime.now(timezone.utc).isoformat(),
    "profile": profile,
    "mesh_revision": revision,
    "source_date_epoch": int(epoch),
    "target": target,
    "toolchain": {"rustc": rustc, "cargo": cargo},
    "tools": {"cargo_audit": audit_tool, "cargo_cyclonedx": cyclonedx_tool},
    "accepted_named_licenses": ["Apache-2.0/MIT", "MIT/Apache-2.0"],
    "results": {
        "dependency_audit": {"status": "passed", "sha256": audit_sha},
        "sbom": {"status": "passed", "format": "CycloneDX 1.5 JSON", "sha256": sbom_sha},
        "secure_equals_timing": {"status": "passed", "sha256": timing_sha},
        "reproducible_meshc_build": {"status": "passed", "sha256": artifact_sha},
    },
    "known_limitations": [
        "This record does not provide fuzz coverage or the complete secret-leak sentinel suite.",
        "This record covers the current host target only, not the advertised mobile and host matrix.",
        "This record is not an independent cryptographic, protocol, server, or mobile security review.",
    ],
}

with open(path, "w", encoding="utf-8") as output:
    json.dump(record, output, indent=2, sort_keys=True)
    output.write("\n")
PY

(
  cd "${OUTPUT_DIR}"
  for evidence_file in cargo-audit.json meshc.cdx.json constant-time.json reproducibility.json release-record.json; do
    printf '%s  %s\n' "$(sha256_file "${evidence_file}")" "${evidence_file}"
  done
) >"${OUTPUT_DIR}/SHA256SUMS"

[[ "$(git rev-parse --verify HEAD)" == "${REVISION}" ]] || fail "repository revision changed during evidence generation"
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || fail "tracked worktree changed during evidence generation"

printf 'release evidence generated: %s\n' "${OUTPUT_DIR}"
