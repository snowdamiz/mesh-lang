# PostgreSQL Binary Values

`DbValue` distinguishes database text, binary, and null values:

```mesh
let params = [Binary(ciphertext), Text("pending"), Null]
let _ = Pool.execute_values(pool, sql, params) ?
let rows = Pool.query_values(pool, query, [Binary(mailbox_hash)]) ?

case Map.get(List.head(rows), "ciphertext") do
  Binary(bytes) -> Bytes.length(bytes)
  Text(_) -> 0
  Null -> 0
end
```

`Pg.execute_values/query_values` provide the same API on an affine connection
from `Pg.connect`. Existing `Pg/Pool.execute` and `query` keep their `List<String>`
and `Map<String, String>` behavior.

The typed APIs use PostgreSQL's extended query protocol and unnamed prepared
statements. `Binary` parameters use wire format 1; they are never converted to
UTF-8, hex, or base64. Queries describe the prepared statement first, request
binary results only for PostgreSQL `BYTEA` (OID 17), and keep all other columns
in text format. SQL NULL becomes `Null`, so it is distinct from empty text and
empty bytes.

Safety limits are 16 MiB per parameter/result cell, 64 MiB per wire message,
64 MiB of decoded typed-result allocations, 32,767 parameters/columns, and
100,000 result rows. Limit violations drain to `ReadyForQuery` where possible;
wire and framing failures mark the connection unusable so a pool discards it.

The public round-trip proof is ignored by default because it needs PostgreSQL:

```sh
MESH_TEST_DATABASE_URL='postgres://mesh_test:mesh_test@127.0.0.1:5432/mesh_test?sslmode=disable' \
  cargo test -p meshc --test e2e_db_values \
  postgres_bytea_round_trips_through_public_mesh_api -- --ignored --exact
```
