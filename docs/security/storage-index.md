# Storage-key index tags

`StorageKey.index_tag(key, namespace, value) -> Bytes!CryptoError` borrows an
actor-owned `StorageKey` and returns exactly 32 bytes. The flat alias is
`storage_key_index_tag`. Namespace length is 1 through 256 bytes; value length is
0 through 65,536 bytes. Invalid lengths return `InvalidLength(maximum, actual)`;
the namespace maximum is 256 even for an empty namespace. The key remains usable
for indexing and sealing. Indexing does not reserve or advance a sealing counter.

The exact construction is:

```
K_index = HKDF-SHA256(IKM = storage_master_key[32],
                     salt = UTF8("mesh/storage-index/v1"),
                     info = namespace, length = 32)
tag = HMAC-SHA256(key = K_index, message = value)
```

The storage nonce prefix and durable counter are not inputs. The master key and
derived key never become Mesh `Bytes`; the temporary derived key is zeroized.
Existing actor ownership, resource-kind, generation and destruction checks apply.
Wrong-kind handles return `InvalidKey`; stale or foreign handles return
`SecretDestroyed`. The API uses the existing platform crypto provider operations
and introduces no provider or dependency.

Applications choose a versioned namespace and canonical value format. Including
a public random database identifier in the namespace separates indexes in two
databases using the same key. Hashing an existing canonical label before calling
this API also permits migration from an unkeyed digest index without recovering
the original labels. Migration authentication, atomicity, collision refusal,
rollback/replay policy and removal of old tables remain application duties.

Tags reveal equality within a key/namespace and provide a chosen-input PRF oracle
to authorized code holding that key. They do not encrypt record data or hide row
counts, lengths, access patterns, or old database/WAL/backup copies. They are not
an authorization credential. Plaintext tracking propagates input dependencies
through this API; it is not a declassification boundary.

The focused independent vector uses a synthetic 32-byte master key consisting
of `31` hex, namespace `636174616c6f67007631` (`catalog\0v1`), and value
`00ff70726f66696c652f7631`. The output is
`2132ed67a6facbd7dc9d08cb94d2efdf0c9169eea6e05d39d2b28f1e3766400b`.
The host runtime test compares this against an independently calculated
HKDF/HMAC vector, tests exact bounds, domain separation, ownership and exhausted
sealing-counter behavior. Compiled Mesh tests exercise direct, flat and pipe
calls with repeated key use at optimization levels 0 and 2. These host checks
do not constitute native-target or independent security review.
