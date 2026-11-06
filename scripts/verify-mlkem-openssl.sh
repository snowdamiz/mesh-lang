#!/usr/bin/env bash
# Explicit external-tool gate: never treat zero selected tests as success.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root}"
log="$(mktemp "${TMPDIR:-/tmp}/mesh-mlkem-openssl.XXXXXX")"
trap 'rm -f -- "${log}"' EXIT
test_name=crypto::mlkem_conformance_tests::c6_openssl_mlkem768_deterministic_interoperability_and_implicit_rejection
if ! cargo test --locked -p mesh-rt --lib "${test_name}" \
    -- --ignored --exact --nocapture --test-threads=1 >"${log}" 2>&1; then
  cat "${log}"
  exit 1
fi
cat "${log}"
grep -Fq 'test result: ok. 1 passed; 0 failed; 0 ignored;' "${log}"
grep -Fq 'C6_OPENSSL=OpenSSL ' "${log}"
grep -Fxq 'C6_OPENSSL_INTEROP=passed' "${log}"
