# Blind RSA conformance vectors

These are published test keys, including private parameters. They are public
and unsafe for any real issuance. No generated application key belongs here.

- [`rfc9578-type2.json`](rfc9578-type2.json) transcribes all five vectors from
  [RFC 9578 Appendix A.2](https://www.rfc-editor.org/rfc/rfc9578.html#appendix-A.2).
  Original field names and octets are retained. `pkcs8_der` decodes the
  published `skI` PEM. `token_input`, `authenticator` and `blinded_msg` split
  the published token/request at the protocol's fixed boundaries. The extractor
  checks the challenge hash, key ID, nonce and truncated key ID independently.
- [`rfc9474-appendix-a.json`](rfc9474-appendix-a.json) transcribes
  [RFC 9474 Appendix A.1–A.4](https://www.rfc-editor.org/rfc/rfc9474.html#appendix-A).
  These 4,096-bit vectors exercise internal helpers and deterministic test
  entropy. They do not expand BR1 public acceptance beyond 2,048-bit RSA,
  exponent 65,537, SHA-384, MGF1-SHA-384, 48-byte salt and identity preparation.

The source records contain SHA-256 hashes of the RFC Editor text files. To
reproduce both JSON files from unmodified downloads:

```sh
python3 tests/vectors/blind-rsa/import-rfc-vectors.py /path/rfc9578.txt /path/rfc9474.txt
```

The RFCs are copyright the IETF Trust and their named authors and are subject
to [BCP 78 and the IETF Trust Legal Provisions](https://trustee.ietf.org/license-info).
These numeric test data are transcribed with source attribution; no RFC prose
or implementation is copied.

`compiler/meshc/tests/e2e_crypto_v2.rs` compiles the public Mesh API fixture,
checks all five Type 2 signatures and exact blind signing responses, exercises
fresh production-randomness blind/sign/finalize, and compares exported public
SPKI/signatures with the OpenSSL CLI. Private keys enter Mesh only through
`Env.get_secret_hex` into `SecretBytes`. Exact deterministic blind/finalize
vectors use the runtime's compile-time test/fuzz-only provider; production has no
provider-selection or entropy-injection switch.

The raw RSA oracle uses `openssl pkeyutl -decrypt -pkeyopt
rsa_padding_mode:none`, the same private exponentiation as raw signing. The
OpenSSL CLI's `-sign` command rejects modulus-width inputs at its digest-size
check even with padding disabled ([upstream issue 30777](https://github.com/openssl/openssl/issues/30777)).
This oracle performs no hashing, padding or algorithm substitution.
