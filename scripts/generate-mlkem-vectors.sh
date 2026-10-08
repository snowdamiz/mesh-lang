#!/usr/bin/env bash
set -euo pipefail

# Regenerate the ML-KEM-768 vectors the runtime tests check its provider
# against (crypto::tests in compiler/mesh-rt):
#
#   tests/vectors/mlkem/mlkem768-acvp-fips203.json
#     NIST ACVP FIPS 203 internal projections at a pinned ACVP-Server commit:
#     every ML-KEM-768 key generation (d, z -> ek, dk), encapsulation
#     (ek, m -> c, k) and decapsulation (dk, c -> k, including modified
#     ciphertexts that must take the implicit-rejection path) case.
#   tests/vectors/mlkem/mlkem768-openssl.json
#     A differential against OpenSSL, which shares no code with the runtime's
#     provider: OpenSSL derives the public key from each seed, encapsulates
#     with a fixed m, and decapsulates the ciphertext and a tampered copy.
#
# usage: bash scripts/generate-mlkem-vectors.sh
# Needs curl, python3 and OpenSSL 3.5 or later (MESH_OPENSSL selects it).

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT_DIR
readonly OUTPUT_DIR="${ROOT_DIR}/tests/vectors/mlkem"
readonly ACVP_COMMIT="65370b861b96efd30dfe0daae607bde26a78a5c8"
readonly ACVP_BASE="https://raw.githubusercontent.com/usnistgov/ACVP-Server/${ACVP_COMMIT}/gen-val/json-files"
readonly OPENSSL="${MESH_OPENSSL:-openssl}"
readonly OPENSSL_CASES=8

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

curl -sfL "${ACVP_BASE}/ML-KEM-keyGen-FIPS203/internalProjection.json" -o "${work}/keygen.json"
curl -sfL "${ACVP_BASE}/ML-KEM-encapDecap-FIPS203/internalProjection.json" -o "${work}/encdec.json"

python3 - "${work}" "${OUTPUT_DIR}/mlkem768-acvp-fips203.json" "${ACVP_COMMIT}" "${ACVP_BASE}" <<'PY'
import json, sys
work, output, commit, base = sys.argv[1:]
keygen = json.load(open(f"{work}/keygen.json"))
encdec = json.load(open(f"{work}/encdec.json"))

def group(document, function):
    [found] = [g for g in document["testGroups"]
               if g["parameterSet"] == "ML-KEM-768" and g.get("function", function) == function]
    return found

def pick(test, *keys):
    # ACVP writes upper-case hex; the other Mesh vectors use lower case.
    return {k: test[k].lower() if k not in ("tcId", "reason") else test[k] for k in keys}

kg, enc, dec = group(keygen, "keyGen"), group(encdec, "encapsulation"), group(encdec, "decapsulation")
json.dump({
    "schema_version": 1,
    "suite": "ML-KEM-768",
    "version": "FIPS 203",
    "source": {
        "name": "NIST ACVP ML-KEM FIPS203 internal projections",
        "commit": commit,
        "key_generation_url": f"{base}/ML-KEM-keyGen-FIPS203/internalProjection.json",
        "encapsulation_decapsulation_url": f"{base}/ML-KEM-encapDecap-FIPS203/internalProjection.json",
        "test_group_ids": {"key_generation": kg["tgId"], "encapsulation": enc["tgId"],
                           "decapsulation": dec["tgId"]},
    },
    "key_generation": [pick(t, "tcId", "d", "z", "ek", "dk") for t in kg["tests"]],
    "encapsulation": [pick(t, "tcId", "ek", "m", "c", "k") for t in enc["tests"]],
    "decapsulation": {
        "dk": dec["dk"].lower(),
        "cases": [pick(t, "tcId", "reason", "c", "k") for t in dec["tests"]],
    },
}, open(output, "w"), indent=2)
open(output, "a").write("\n")
PY

hex() { od -An -v -tx1 "$1" | tr -d ' \n'; }

version="$("${OPENSSL}" version)"
cases=()
for index in $(seq 0 $((OPENSSL_CASES - 1))); do
  seed="$("${OPENSSL}" rand -hex 64)"
  m="$("${OPENSSL}" rand -hex 32)"
  "${OPENSSL}" genpkey -algorithm ML-KEM-768 -pkeyopt "hexseed:${seed}" -out "${work}/sk.pem"
  "${OPENSSL}" pkey -in "${work}/sk.pem" -pubout -outform DER -out "${work}/pk.der"
  "${OPENSSL}" pkey -in "${work}/sk.pem" -pubout -out "${work}/pk.pem"
  "${OPENSSL}" pkeyutl -encap -pubin -inkey "${work}/pk.pem" -pkeyopt "hexikme:${m}" \
    -out "${work}/c.bin" -secret "${work}/k.bin"
  # Flip one bit, at a different position each case, for implicit rejection.
  python3 - "${work}/c.bin" "${work}/tampered.bin" "$((index * 137 % 1088))" <<'PY'
import sys
data = bytearray(open(sys.argv[1], "rb").read())
data[int(sys.argv[3])] ^= 1
open(sys.argv[2], "wb").write(data)
PY
  "${OPENSSL}" pkeyutl -decap -inkey "${work}/sk.pem" -in "${work}/tampered.bin" \
    -secret "${work}/rejected.bin"
  # An ML-KEM-768 SubjectPublicKeyInfo is a 22-byte header and the raw key.
  [[ "$(wc -c <"${work}/pk.der" | tr -d ' ')" == 1206 ]] || {
    printf 'unexpected ML-KEM-768 SPKI length\n' >&2
    exit 1
  }
  tail -c 1184 "${work}/pk.der" >"${work}/ek.bin"
  cases+=("$(printf '{"seed":"%s","m":"%s","ek":"%s","c":"%s","k":"%s","tampered_c":"%s","rejected_k":"%s"}' \
    "${seed}" "${m}" "$(hex "${work}/ek.bin")" "$(hex "${work}/c.bin")" "$(hex "${work}/k.bin")" \
    "$(hex "${work}/tampered.bin")" "$(hex "${work}/rejected.bin")")")
done

python3 - "${OUTPUT_DIR}/mlkem768-openssl.json" "${version}" "${cases[@]}" <<'PY'
import json, sys
output, version, *cases = sys.argv[1:]
json.dump({
    "schema_version": 1,
    "suite": "ML-KEM-768",
    "version": "FIPS 203",
    "source": {
        "name": version,
        "commands": [
            "openssl genpkey -algorithm ML-KEM-768 -pkeyopt hexseed:<seed>",
            "openssl pkeyutl -encap -pubin -inkey <public key> -pkeyopt hexikme:<m>",
            "openssl pkeyutl -decap -inkey <private key> -in <tampered_c>",
        ],
    },
    "cases": [json.loads(case) for case in cases],
}, open(output, "w"), indent=2)
open(output, "a").write("\n")
PY

printf 'wrote %s and %s\n' "${OUTPUT_DIR}/mlkem768-acvp-fips203.json" "${OUTPUT_DIR}/mlkem768-openssl.json"
