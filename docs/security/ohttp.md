# Bounded OHTTP request and response operations

The public `Ohttp` module implements RFC 9458 request/reply encapsulation for
DHKEM(X25519, HKDF-SHA256), HKDF-SHA256 and ChaCha20-Poly1305 (suite 32/1/3).
It uses the RFC 9180 base-mode key schedule. It does not parse Binary HTTP,
select a trusted configuration, authorize a decrypted request, route HTTP,
or establish relay/gateway operator independence.

Both qualified names below and the corresponding flat `ohttp_*` names exist:

```
Ohttp.encapsulate_request(key_id :: Int, recipient :: X25519PublicKey,
                          plaintext :: Bytes) -> OhttpRequest!CryptoError
Ohttp.decapsulate_request(key :: borrow X25519PrivateKey, key_id :: Int,
                          wire :: Bytes) -> OhttpRequestOpening!CryptoError
Ohttp.encapsulate_response(context :: consume OhttpGatewayContext,
                           plaintext :: Bytes) -> Bytes!CryptoError
Ohttp.decapsulate_response(context :: consume OhttpClientContext,
                           wire :: Bytes) -> Bytes!CryptoError
```

`OhttpRequest` contains `wire :: Bytes` then
`response_context :: OhttpClientContext`. `OhttpRequestOpening` contains
`plaintext :: Bytes` then `response_context :: OhttpGatewayContext`.
They are affine records. Read any ordinary fields needed before moving the
context according to the existing resource rules. The gateway private key is
borrowed and remains usable; it is revalidated before opened data is returned.
The expected key ID is mandatory and must match the authenticated request.
IDs outside 0..255 return `InvalidLength(255, actual)` without truncation.

Each context is an opaque actor-owned one-shot resource. The client context
opens only its bound response; the gateway context seals only that response.
They cannot be printed, serialized, persisted, transferred to another actor,
or borrowed into an operation that consumes them. `Secret.destroy` and actor
exit use the existing erasure path. There is no context constructor, exporter,
or storage purpose. Do not pass contexts between separate native library calls:
each call owns its own actor lifetime.

A response operation first validates the current actor and expected role. A
wrong owner/role does not consume the rightful resource. A valid owned role
is consumed before subsequent fallible work, including parsing a null or
oversize FFI byte input. Authentication, entropy, provider, or input refusal
therefore cannot leave that response context reusable. A fresh request creates
a fresh context; retrying an uncertain effect is a caller protocol decision,
not permission to recreate an already-used response context.

The independent maximum is 1,048,576 bytes for the complete OHTTP wire. Request
plaintext is at most 1,048,521 bytes (7-byte header +32-byte encapsulated key
+16-byte AEAD tag); response plaintext is at most 1,048,528 bytes (32-byte
response nonce +16-byte tag). Empty messages are valid. This does not increase
the generic HPKE 64 KiB or AEAD 128 KiB plaintext bounds. Retained context
material is a protected 64-byte response secret/encapsulation binding; the
existing platform protection and provider-temporary limitations still apply.

Only successful encapsulated bytes cross the plaintext encryption boundary.
The request's successful wire field is ciphertext; its opaque context is not
an ordinary-byte declassification API. Public key selection, key ID and caller
control remain dependencies. Failure tags and error payloads, including input
lengths, retain private dependencies. A reviewed `declassify_discriminant` call
changes only its finite result tag, not error payloads or caller control.
Opening retains any annotated input dependencies; this API does not label an
unannotated ingress or make an application a complete plaintext-safe system.
User-defined functions with matching names retain their own summaries.

Request key derivation binds `"message/bhttp request" || 0x00 || header`, and
uses HPKE sequence zero with empty AAD. Response key derivation uses the HPKE
exporter label `"message/bhttp response"`, salt `enc || response_nonce`, and
ordinary HKDF-Expand labels `"key"` and `"nonce"`; it is not HPKE LabeledExpand.
Wire order and these domains are fixed. The selected-suite complete-wire test
comes from an independently pinned reference implementation; RFC 9458
Appendix A selects AES-128 and is not a ChaCha20-Poly1305 known-answer vector.

See [RFC 9458 sections 4.1–4.4](https://www.rfc-editor.org/rfc/rfc9458.html#section-4)
and [RFC 9180 sections 5.1–5.3](https://www.rfc-editor.org/rfc/rfc9180.html#section-5).
API exposure alone does not qualify platform binaries, application transports,
configuration distribution, deployment, or public release.
