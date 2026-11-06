# User-held recovery codes

Development API. Native and release qualification remain separate.

`RecoveryCode.generate() -> (SecretBytes, String)!CryptoError` creates a new
256-bit recovery secret and its display code from operating-system randomness.
`RecoveryCode.parse(code :: String) -> SecretBytes!CryptoError` validates and
imports that bounded representation. The equivalent flat names are
`recovery_code_generate` and `recovery_code_parse`.

These functions support recovery-code setup and restore without exporting an
existing key. Generation takes no input key; there is no redisplay or generic
secret exporter. The returned secret has normal actor ownership, affine use,
automatic destruction and protected-memory limits. Parsing errors do not publish
a partial secret. Neither method logs code contents.

The display String intentionally contains recovery material. Show it only in the
user's recovery setup, require an acknowledgement, then clear the UI. Do not log,
persist or send it to a service. Native/JavaScript/String copies do not provide
prompt zeroization. The API cannot prevent a caller retaining or copying the
String, so showing it once is an application obligation, not a runtime promise.

The canonical v1 text is 85 ASCII bytes: `MRC1-` followed by nine groups of eight
uppercase hexadecimal digits separated by hyphens. The first eight groups hold
the 32-byte secret. The last is the first four bytes of SHA-256 over the exact
ASCII domain `mesh/recovery-code/v1` followed by the secret. This checksum detects
transcription mistakes; it does not authenticate an attacker-chosen code.
Lowercase, whitespace, changed versions/separators, non-ASCII, wrong lengths,
trailing data and wrong checksums are rejected with `InvalidKey`. UI text
normalization, if desired, must happen explicitly before parsing.

Keep the returned secret opaque when deriving protocol keys. For example, a
backup's versioned Argon2id profile remains a separate policy; this API adds no
weak fallback KDF. Decrypting a backup does not itself authorize account access
or another device. The application must enforce those separate policies.
