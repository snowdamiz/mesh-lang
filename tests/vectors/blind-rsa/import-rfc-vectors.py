#!/usr/bin/env python3
"""Reproduce the public RFC fixtures from unmodified RFC Editor text files.

Usage: python3 import-rfc-vectors.py /path/rfc9578.txt /path/rfc9474.txt
All private parameters here are intentionally published test keys, never secrets.
"""
import base64
import hashlib
import json
from pathlib import Path
import re
import sys


def fields(block, separator):
    result = {}
    field = None
    for line in block.splitlines():
        match = re.fullmatch(r"   ([A-Za-z_]+)\s*" + separator + r"\s*([0-9a-f]*)", line)
        if match:
            field, value = match.groups()
            result[field] = value
        elif field and re.fullmatch(r"   [0-9a-f]+", line):
            result[field] += line.strip()
        else:
            field = None
    for key, value in result.items():
        bytes.fromhex(value)  # Reject accidental prose or incomplete octets.
    return result


def save(name, record):
    (Path(__file__).parent / name).write_text(json.dumps(record, indent=2) + "\n")


pp_source = Path(sys.argv[1]).read_bytes()
pp = pp_source.decode("utf-8-sig").rsplit("A.2.  Issuance Protocol 2 - Blind RSA (2048-bit)\n", 1)[1]
pp_vectors = []
for index, block in enumerate(re.split(r"   // Test vector [1-5]\n", pp)[1:], 1):
    vector = fields(block, ":")
    assert set(vector) == {"skI", "pkI", "token_challenge", "nonce", "blind", "salt", "token_request", "token_response", "token"}
    pem = bytes.fromhex(vector["skI"]).decode()
    encoded_key = "".join(line for line in pem.splitlines() if not line.startswith("---"))
    vector["pkcs8_der"] = base64.b64decode(encoded_key, validate=True).hex()
    token = bytes.fromhex(vector["token"])
    assert len(token) == 354 and token[:2] == b"\0\2"
    assert token[2:34].hex() == vector["nonce"]
    assert token[34:66] == hashlib.sha256(bytes.fromhex(vector["token_challenge"])).digest()
    assert token[66:98] == hashlib.sha256(bytes.fromhex(vector["pkI"])).digest()
    request = bytes.fromhex(vector["token_request"])
    assert len(request) == 259 and request[:2] == b"\0\2" and request[2] == token[97]
    assert len(bytes.fromhex(vector["token_response"])) == 256
    modulus = int.from_bytes(bytes.fromhex(vector["pkI"])[81:337], "big")
    response = int(vector["token_response"], 16)
    assert pow(response, 65537, modulus) == int.from_bytes(request[3:], "big")
    assert response * pow(int(vector["blind"], 16), -1, modulus) % modulus == int.from_bytes(token[98:], "big")
    vector.update(id=index, token_input=token[:98].hex(), authenticator=token[98:].hex(), blinded_msg=request[3:].hex())
    pp_vectors.append(vector)
assert len(pp_vectors) == 5
save("rfc9578-type2.json", {
    "schema_version": 1, "suite": "RSABSSA-SHA384-PSS-Deterministic", "profile": "BR1",
    "source": {"url": "https://www.rfc-editor.org/rfc/rfc9578.txt", "section": "Appendix A.2", "sha256": hashlib.sha256(pp_source).hexdigest()},
    "notice": "Private issuer keys are PUBLIC RFC test keys. Never use them outside tests. Added pkcs8_der decodes skI PEM; token_input/authenticator/blinded_msg split published wire values.",
    "vectors": pp_vectors,
})

rsa_source = Path(sys.argv[2]).read_bytes()
rsa = rsa_source.decode("utf-8-sig")
rsa_vectors = []
for index in range(1, 5):
    match = re.search(r"^A\." + str(index) + r"\.  ([^\n]+) Test Vector\n", rsa, re.MULTILINE)
    assert match
    end = re.search(r"^(A\.[1-4]\.|Acknowledgments)", rsa[match.end():], re.MULTILINE)
    block = rsa[match.end(): match.end() + end.start()] if end else rsa[match.end():]
    vector = fields(block, "=")
    assert {"p", "q", "n", "e", "d", "msg", "prepared_msg", "salt", "encoded_msg", "inv", "blinded_msg", "blind_sig", "sig"} <= set(vector)
    assert int(vector["p"], 16) * int(vector["q"], 16) == int(vector["n"], 16)
    assert pow(int(vector["blinded_msg"], 16), int(vector["d"], 16), int(vector["n"], 16)) == int(vector["blind_sig"], 16)
    assert int(vector["blind_sig"], 16) * int(vector["inv"], 16) % int(vector["n"], 16) == int(vector["sig"], 16)
    vector.update(id=index, suite=match.group(1), modulus_bits=int(vector["n"], 16).bit_length())
    rsa_vectors.append(vector)
save("rfc9474-appendix-a.json", {
    "schema_version": 1,
    "source": {"url": "https://www.rfc-editor.org/rfc/rfc9474.txt", "section": "Appendix A.1-A.4", "sha256": hashlib.sha256(rsa_source).hexdigest()},
    "notice": "PUBLIC test private parameters; never use outside tests. These 4096-bit variants exercise internal helpers, not the BR1 public key acceptance profile.",
    "vectors": rsa_vectors,
})
print(f"extracted {len(pp_vectors)} RFC9578 Type2 and {len(rsa_vectors)} RFC9474 AppendixA vectors")
