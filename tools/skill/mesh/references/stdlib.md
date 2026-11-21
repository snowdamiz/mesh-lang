# Find standard-library APIs

Call compiler-recognized modules directly, for example `Bytes.length(data)`.
Check the exact function signature and error behavior in the linked guide.
For the complete module inventory and official package index, use the
[language reference](https://meshlang.dev/docs/reference/#standard-library-module-index).

| Task | Modules and types | Reference |
| --- | --- | --- |
| Work with text, regex, and JSON | `String`, `Regex`, `Json` (`JSON` is an alias) | `meshc guide strings`, [text and regex](https://meshlang.dev/docs/stdlib/#strings), [JSON](https://meshlang.dev/docs/web/#json) |
| Transform collections | `List`, `Map`, `Set`, `Tuple`, `Range`, `Queue`, `Iter` | `meshc guide collections`, [iterators](https://meshlang.dev/docs/iterators/) |
| Work with binary data and integers | `Bytes`, `BytesBuilder`, `Base64`, `Hex`, `U64`, `U128`, `I128`, `Checked`, `Math`, `Int`, `Float` | [bytes](https://meshlang.dev/docs/stdlib/#bytes), [wide integers](https://meshlang.dev/docs/stdlib/#wide-integers), [encoding](https://meshlang.dev/docs/stdlib/#encoding) |
| Read input, files, environment, or time | `IO`, `Env`, `File`, `DateTime`, `Monotonic`, `Duration`, `Random` | [system APIs](https://meshlang.dev/docs/stdlib/#input-environment-and-files), [time](https://meshlang.dev/docs/stdlib/#datetime), [deterministic randomness](https://meshlang.dev/docs/stdlib/#deterministic-randomness) |
| Work with protected data and keys | `Crypto`, `Secret`, `SecretMap`, `StorageKey`, `X25519PrivateKey`, `SigningPrivateKey`, `MlKemPrivateKey`, `BlindRsaSecretKey`, `Plaintext` | [cryptography](https://meshlang.dev/docs/stdlib/#crypto), [resources and plaintext](https://meshlang.dev/docs/type-system/#plaintext) |
| Access host capabilities from a library | `Host` | [host capabilities](https://meshlang.dev/docs/stdlib/#host-capabilities), [library builds](https://meshlang.dev/docs/library-builds/) |
| Schedule local work and communicate | `Job`, `Timer`, `Channel`, `Process`, `Test` | `meshc guide actors`, [concurrency](https://meshlang.dev/docs/concurrency/), [testing](https://meshlang.dev/docs/testing/) |
| Handle network traffic | `HTTP`, `Request`, `Ws`, `Http`, `WsClient` | `meshc guide http`, [web and networking](https://meshlang.dev/docs/web/) |
| Query and migrate databases | `Sqlite`, `Pg`, `Pool`, `Orm`, `Expr`, `Query`, `Repo`, `Changeset`, `Migration` | `meshc guide database`, [databases](https://meshlang.dev/docs/databases/) |
| Distribute actors and inspect a cluster | `Node`, `Global`, `Continuity`, `Cluster` | `meshc guide clustering`, [distributed actors](https://meshlang.dev/docs/distributed/) |

Use `Bytes` for binary values and `String` for text. `BytesBuilder` lets
you append chunks before producing a `Bytes` value. Use the documented
checked integer APIs when overflow needs to return an error.

Use the type system's resource and plaintext rules when passing secrets,
keys, or plaintext between functions, actors, and host code. Read the
operation's ownership signature before reusing a value. For formatting,
serialization, explicit declassification, and host display exports, follow
the [resource rules](https://meshlang.dev/docs/type-system/#resource-types) and
[plaintext rules](https://meshlang.dev/docs/type-system/#plaintext).
