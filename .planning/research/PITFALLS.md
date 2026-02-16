# Domain Pitfalls

**Domain:** ORM library for the Mesh programming language -- schema DSL, query builder, relationships, migrations, changesets, targeting PostgreSQL
**Researched:** 2026-02-16
**Confidence:** HIGH (Mesh compiler source analysis + Ecto/Diesel/Persistent architecture research + Mesher dogfooding experience)

---

## Critical Pitfalls

Mistakes that cause rewrites, block progress for days, or produce fundamentally broken ORM behavior.

---

### Pitfall 1: Schema DSL Without Macros -- The Ecto Trap

**What goes wrong:** The developer tries to replicate Ecto's schema DSL syntax:
```elixir
schema "users" do
  field :name, :string
  field :email, :string
  field :age, :integer
  timestamps()
end
```
In Ecto, this works because `schema`, `field`, and `timestamps` are Elixir macros that expand at compile time into struct definitions, type metadata, changeset casting rules, and query builder column references. Without macros, there is no mechanism to transform declarative `field :name, :string` calls into compile-time struct field definitions, type mappings, and codegen hooks.

**Why it happens:** Ecto's entire schema system is macro-powered. The `schema` macro defines the Elixir struct, the `field` macro registers field metadata in module attributes, and the `timestamps` macro injects `inserted_at`/`updated_at` fields. Every ORM in a macro-capable language (Ecto, ActiveRecord DSL, Diesel's `table!`) relies on compile-time code transformation. Mesh has no macros, no runtime reflection, and no code-as-data -- the three mechanisms other ORMs depend on.

**Consequences:**
- Attempting to build a runtime DSL that "configures" schemas at startup produces stringly-typed metadata with no compile-time safety
- A purely runtime approach cannot generate struct definitions -- structs must be defined at compile time in Mesh
- Trying to bolt on a "mini macro system" for this one use case adds enormous compiler complexity and creates a second compilation model

**Prevention:**
1. **Use `deriving(Schema)` as the single entry point.** Follow the established `deriving(Json)` and `deriving(Row)` pattern -- the compiler already has infrastructure to generate code from struct definitions at MIR lowering time. The struct definition IS the schema:
   ```
   struct User do
     id :: Int
     name :: String
     email :: String
     age :: Option<Int>
   end deriving(Schema, Json)
   ```
   The `deriving(Schema)` generates: table name (pluralized snake_case of struct name), column metadata map, from_row function (subsuming `deriving(Row)`), to_params function (struct to parameter list), and relationship accessor functions.

2. **Add schema metadata as compiler-generated constants**, not runtime configuration. The compiler can emit a `__schema__` module-level constant containing table name, column names, column types, and relationship metadata. This is the same approach Diesel uses (generating a Rust module per table) without requiring macros at the Mesh language level -- the compiler itself is the code generator.

3. **Do NOT attempt to build a runtime schema registry.** Runtime registries require mutable global state (Mesh has no mutable variables), reflection (Mesh has no reflection), and initialization ordering guarantees (Mesh has no static initializers). The compiler is the right place for this.

**Detection:** If schema definitions require runtime function calls to register metadata, the approach is wrong. Schema information should be fully resolved at compile time.

**Phase mapping:** Must be the first phase. Every subsequent ORM feature depends on schema metadata being available at compile time. Getting this wrong means rewriting everything.

**Compiler additions needed:** New `deriving(Schema)` implementation in `lower_struct_def` following the `deriving(Json)`/`deriving(Row)` pattern. Extend `valid_derives` in `infer.rs` (line 2276). Generate synthetic MIR functions for table name, column metadata, and from_row/to_params conversions.

---

### Pitfall 2: Query Builder Type Safety -- The String Concatenation Trap

**What goes wrong:** The query builder is implemented as string concatenation:
```
fn where(query, column, value) do
  Query { sql: query.sql <> " WHERE " <> column <> " = $" <> next_param(query), params: List.append(query.params, value) }
end
```
This compiles and runs, but there is zero type safety: `column` is an unchecked string that could be `"name"`, `"nonexistent_column"`, or `"; DROP TABLE users --"`. The query builder becomes a SQL string builder with parameterized values but unvalidated column references.

**Why it happens:** Without macros or compile-time code generation, column names cannot be validated against the schema at compile time. Diesel solves this with Rust's type system (each column is a unique type, joins are checked via trait bounds). Ecto solves it with macros (column references in `from u in User, where: u.name == ^name` are validated by the macro). Mesh has neither mechanism available at the language level.

**Consequences:**
- Column name typos (`"nme"` instead of `"name"`) produce runtime SQL errors, not compile-time errors
- SQL injection through column name injection if user input reaches the column parameter
- Refactoring a column name requires finding every string reference manually -- no compiler assistance
- The ORM provides no more safety than raw `Pool.query` calls with handwritten SQL

**Prevention:**
1. **Generate column accessor functions per schema field.** When `deriving(Schema)` processes a struct with field `name :: String`, the compiler generates a function `User.name_col() -> String` that returns `"name"`. Query builder functions accept these column accessor return values. This makes column references function calls, which the compiler can type-check.

2. **Better: generate column accessor structs or use field name strings from the schema metadata.** The compiler knows all column names at compile time. Generate a `User.__columns__` map or a set of accessor functions. The query builder accepts `Column` values produced by these accessors, not arbitrary strings.

3. **Separate parameterized values from structural SQL.** Values use `$1, $2` parameterization (already implemented). Column names, table names, and SQL keywords are compile-time constants embedded in the generated SQL string. User input should never flow into the structural parts of the query.

4. **Accept that Mesh cannot achieve Diesel-level compile-time query validation** without significant type system extensions. The practical goal is: column references are compiler-generated strings (not user input), values are parameterized (no injection), and type mismatches between Mesh types and SQL types are caught by `deriving(Schema)` validation.

**Detection:** If `where()` accepts an arbitrary `String` for the column name, the design is wrong. Column references should come from compiler-generated functions or constants.

**Phase mapping:** Query builder phase. Must be designed after schema metadata generation is working, because column accessors depend on schema information.

---

### Pitfall 3: The N+1 Problem Without Lazy Loading -- Preload Design Failure

**What goes wrong:** The developer writes code that loads users and then iterates to load each user's posts:
```
let users = User |> Repo.all()
List.map(users, fn(user) do
  let posts = Post |> where("user_id", user.id) |> Repo.all()
  {user, posts}
end)
```
This executes N+1 queries (1 for users, N for posts). In ORMs with lazy loading (ActiveRecord, Hibernate), this happens invisibly when accessing `user.posts`. In Mesh, it happens explicitly but is still the natural first approach developers take.

**Why it happens:** Mesh has no lazy loading (no mutable variables means no proxy objects that load on access). This is actually a feature -- it prevents invisible N+1 queries. But it means the ORM must provide explicit preloading that is both ergonomic and correct. Ecto's `Repo.preload(users, :posts)` strategy -- issuing one `SELECT * FROM posts WHERE user_id IN (...)` query -- is the right model.

**Consequences:**
- Performance degrades linearly with the number of parent records
- The N+1 pattern is the "natural" code structure (iterate and query), so developers fall into it by default
- Without preload, developers write manual `WHERE ... IN (...)` queries, losing the ORM abstraction
- Nested relationships (users -> posts -> comments) compound the problem to N*M+1 queries

**Prevention:**
1. **Implement Ecto-style preloading with separate queries.** `Repo.preload(users, ["posts"])` collects all user IDs, executes a single `SELECT * FROM posts WHERE user_id IN ($1, $2, ...)`, then maps results back to parent records in memory. This turns N+1 into 2 queries.

2. **Support nested preloading.** `Repo.preload(users, ["posts", "posts.comments"])` loads three total queries regardless of data size. The preloader resolves the dependency order (posts before comments) and batches appropriately.

3. **Do NOT implement lazy loading.** Mesh's immutable variables make lazy loading impossible without fundamental language changes. This is correct -- lazy loading is the source of N+1 problems in other ORMs. Explicit preloading is strictly better.

4. **The string-based relationship name is necessary** because Mesh has no atoms/symbols. `"posts"` as a relationship identifier is the pragmatic choice given Mesh's type system. The compiler can validate these strings against schema relationship metadata at compile time.

5. **Preload must handle the "all values are strings" constraint.** Foreign key values come from `deriving(Row)` as strings. The `IN (...)` clause must use proper `$1::uuid` or `$1::int` casting based on the column type from schema metadata. Getting the cast wrong produces empty result sets or type errors.

**Detection:** Any code pattern that queries inside a `List.map`/`for` loop over records from a previous query is an N+1 problem. The preload API must make the batched alternative as ergonomic as the loop pattern.

**Phase mapping:** Relationship and preloading phase. Must come after basic query builder and schema definitions.

---

### Pitfall 4: deriving(Row) All-Strings Problem Infects the Entire ORM

**What goes wrong:** PostgreSQL's text protocol returns all values as strings. `deriving(Row)` maps `Map<String, String>` to struct fields, parsing `String` -> `Int`, `String` -> `Float`, etc. The ORM must handle:
- Integer primary keys that arrive as `"42"` and need to be stored as `Int` in the struct but sent back as `"42"` in query parameters
- UUID columns that are strings at every layer
- Boolean columns arriving as `"t"` or `"f"` (PostgreSQL text format)
- Timestamp columns arriving as `"2026-02-16 12:00:00+00"` that must remain strings (Mesh has no DateTime type)
- NULL columns arriving as missing keys in the map
- JSONB columns arriving as JSON strings that need `from_json()` parsing

The ORM must maintain a consistent type coercion layer between Mesh types and PostgreSQL text representations at every boundary: insert (Mesh -> SQL params), select (SQL result -> Mesh struct), where clause values, and join conditions.

**Why it happens:** Mesh uses PostgreSQL's text protocol exclusively. The Extended Query protocol returns all column values as text strings. The binary protocol (which returns typed binary representations) would require significant runtime changes. This is a fundamental architectural constraint, not a bug.

**Consequences:**
- Every field type needs bidirectional coercion: Mesh type <-> String (for SQL params/results)
- Type mismatches are silent: inserting `"42"` into an INTEGER column works (PostgreSQL casts it), but inserting `"hello"` into an INTEGER column produces a runtime PostgreSQL error, not a Mesh compile error
- The changeset layer must validate types BEFORE sending to PostgreSQL to catch errors early
- Foreign key matching in preloads must compare strings correctly (is `"42"` == `"42"`? Yes. Is `"42"` == `42`? Depends on representation.)

**Prevention:**
1. **Centralize type coercion in schema metadata.** Each field's schema metadata includes: Mesh type, PostgreSQL column type, to_param function (Mesh value -> SQL string), from_column function (SQL string -> Mesh value). These are generated by `deriving(Schema)`.

2. **Changeset casting validates and coerces in one step.** `Changeset.cast(user, params, ["name", "age"])` takes string parameters (from HTTP request or DB row), validates they can be parsed to the field's Mesh type, and produces a changeset with typed values. Validation errors are collected, not thrown.

3. **Query parameter coercion is automatic.** When the query builder produces `WHERE age = $1`, it calls the field's `to_param` function to convert the Mesh `Int` to the SQL string `"42"`. The developer writes `where("age", 42)` and the ORM handles coercion.

4. **Do NOT add binary protocol support for the ORM.** This is a massive runtime change that is out of scope. The text protocol works -- it just needs a coercion layer. All existing Mesher code uses text protocol successfully.

**Detection:** If ORM insert/select operations produce PostgreSQL type errors at runtime (e.g., "invalid input syntax for type integer"), the coercion layer has a gap.

**Phase mapping:** Schema definition phase (coercion functions) and changeset phase (validation before persistence).

---

### Pitfall 5: Migration File Generation Without String Interpolation for SQL Types

**What goes wrong:** The migration system needs to generate SQL DDL statements from schema definitions:
```sql
CREATE TABLE users (
  id SERIAL PRIMARY KEY,
  name VARCHAR(255) NOT NULL,
  email VARCHAR(255) NOT NULL UNIQUE,
  age INTEGER,
  inserted_at TIMESTAMP NOT NULL DEFAULT now(),
  updated_at TIMESTAMP NOT NULL DEFAULT now()
);
```
This requires mapping Mesh types to PostgreSQL DDL types (`String` -> `VARCHAR(255)`, `Int` -> `INTEGER`, `Option<String>` -> `VARCHAR(255)` without `NOT NULL`). The migration generator must produce correct DDL as a Mesh string, but Mesh has no string interpolation for building multi-line strings, no heredocs, and newlines terminate statements.

**Why it happens:** Mesh's parser treats newlines as statement terminators. Building a multi-line SQL string requires explicit `<>` concatenation across multiple `let` bindings or very long single lines. There is no `"""..."""` heredoc syntax. The migration file itself must be a valid Mesh source file (or a plain SQL file executed by a migration runner).

**Consequences:**
- Migration Mesh code that generates DDL is extremely verbose (one `let` per line of SQL, concatenated)
- Migration files that are plain SQL are simpler but lose type-safety connection to the schema
- The "auto-generate migration from schema diff" feature requires comparing current schema metadata to previous schema metadata -- this is a compiler-level feature, not a library feature

**Prevention:**
1. **Use plain SQL migration files, not Mesh code.** Migration files should be plain `.sql` files in a `migrations/` directory, not `.mpl` files. The migration runner reads and executes them via `Pool.execute`. This avoids the string-building problem entirely.

2. **Migration files are numbered and tracked.** `migrations/001_create_users.sql` (up) and `migrations/001_create_users_down.sql` (down). A `schema_migrations` table tracks which have been applied.

3. **Generate migration SQL from schema diffs as a compiler tool, not a library feature.** The compiler knows the current schema (from `deriving(Schema)` metadata) and can diff it against the database schema (via `information_schema` queries). Output the diff as a SQL file that the developer reviews before applying. This follows the Diesel `diesel print-schema` / Ecto `mix ecto.gen.migration` pattern.

4. **Do NOT attempt to auto-apply migrations.** Generated migrations should always be reviewed by the developer. Auto-applying schema changes is dangerous (see Pitfall 6).

5. **Consider adding a `sql` string literal** to the Mesh lexer/parser for multi-line SQL strings. This is a small compiler addition: `let ddl = sql"CREATE TABLE users (...)"` that allows embedded newlines. Alternatively, support multi-line string literals with a `\` continuation character or triple-quoted strings.

**Detection:** If migration code requires more than 5 lines of string concatenation to produce a single DDL statement, the approach needs simplification (plain SQL files or a multi-line string literal).

**Phase mapping:** Migration phase. Should be one of the later phases since it depends on schema metadata being stable.

**Compiler additions needed:** Either multi-line string literals (`"""..."""` or raw string syntax) or a dedicated migration SQL file reader. Multi-line strings have broader utility beyond just migrations.

---

### Pitfall 6: Migration Rollback Data Loss

**What goes wrong:** A migration adds a column (`ALTER TABLE users ADD COLUMN bio TEXT`), the application runs for a week and populates the column, then a rollback is needed. The down migration runs `ALTER TABLE users DROP COLUMN bio`, permanently destroying a week of user data. There is no recovery path.

Similarly, a migration renames a column from `name` to `full_name`. The up migration works. But the down migration that renames back to `name` succeeds... except the application code was already updated to use `full_name`, so the rollback leaves the app broken because it expects `full_name` but the column is now `name` again.

**Why it happens:** SQL DDL operations are not transactional in all cases (PostgreSQL DDL IS transactional, which helps, but data loss from DROP COLUMN is permanent regardless). Down migrations are rarely tested and often written optimistically. The expand-migrate-contract pattern is not followed.

**Consequences:**
- Data loss from DROP COLUMN / DROP TABLE in rollbacks
- Application crashes after rollback because code expects the new schema
- Rolling deployments where old and new code run simultaneously against the same database fail when migrations change column names or types
- Developers stop writing down migrations because they are untested and dangerous, making the migration system incomplete

**Prevention:**
1. **Design migrations as forward-only by default.** Down migrations should be optional and marked as potentially destructive. The migration runner should warn when running down migrations.

2. **Follow the expand-migrate-contract pattern:**
   - **Expand:** Add the new column/table (non-breaking, old code ignores it)
   - **Migrate:** Deploy new code that uses the new column. Optionally backfill data.
   - **Contract:** Remove the old column/table only after old code is fully decommissioned.

   Each step is a separate migration. The "contract" migration is the only one that can lose data, and it runs days/weeks after the expand.

3. **Never auto-generate DROP COLUMN in down migrations.** The generated down migration for `ADD COLUMN` should be a comment: `-- DROP COLUMN bio (manual review required: will lose data)`. Force the developer to uncomment it intentionally.

4. **Wrap migrations in transactions.** PostgreSQL supports transactional DDL. If a migration fails partway through, the transaction rolls back and the database is unchanged. The migration runner should wrap each migration file in `BEGIN/COMMIT`.

5. **Track migration state in a `schema_migrations` table** with columns `(version, applied_at, checksum)`. The checksum detects if a migration file was modified after application.

**Detection:** Any migration that contains `DROP COLUMN`, `DROP TABLE`, or `ALTER COLUMN ... TYPE` in the up direction should trigger a warning. Any down migration should be carefully reviewed.

**Phase mapping:** Migration phase.

---

### Pitfall 7: Single-Expression Case Arms Break ORM Pattern Matching

**What goes wrong:** ORM operations return `Result<T, String>` at every layer. Natural error handling requires case matching:
```
case Repo.get(User, id) do
  Ok(user) ->
    let posts = Repo.preload(user, ["posts"])
    let json = User.to_json(posts)
    Http.respond(200, json)
  Err(msg) ->
    Http.respond(404, msg)
end
```
This fails in Mesh because case arms are single expressions. The `Ok(user)` arm has three statements (let, let, respond), which the parser rejects.

**Why it happens:** Mesh parser requires single-expression case arms. This is a known constraint documented extensively in the Mesher development (decision [88-02]). Every phase of Mesher encountered this and worked around it with helper function extraction.

**Consequences:**
- Every non-trivial ORM result handling requires extracting a helper function
- Code becomes a patchwork of small helper functions that exist only to satisfy the parser, not for logical decomposition
- The ORM's ergonomic API is undermined because the calling code is forced into an unnatural structure
- Deeply nested Results (query -> parse -> validate -> persist) each need their own helper

**Prevention:**
1. **Add multi-expression case arms to the parser.** This is the single highest-impact compiler improvement for ORM ergonomics. Allow case arms to contain a `do...end` block:
   ```
   case Repo.get(User, id) do
     Ok(user) -> do
       let posts = Repo.preload(user, ["posts"])
       Http.respond(200, User.to_json(posts))
     end
     Err(msg) -> Http.respond(404, msg)
   end
   ```
   This is a parser-level change that does not affect the type system or codegen (the `do...end` block is already supported in other positions).

2. **If the parser is not extended:** Use the `?` operator aggressively to flatten Result chains:
   ```
   fn handle_get_user(id) do
     let user = Repo.get(User, id)?
     let user_with_posts = Repo.preload(user, ["posts"])?
     Ok(Http.respond(200, User.to_json(user_with_posts)))
   end
   ```
   The `?` operator propagates errors automatically, turning multi-step Result handling into a sequence of `let` bindings. This is idiomatic Mesh and avoids the case arm limitation entirely.

3. **Design ORM APIs to be pipe-friendly.** Instead of returning Results that need case matching, provide functions that accept the happy path and propagate errors:
   ```
   let response = User
     |> Repo.get(id)?
     |> Repo.preload(["posts"])?
     |> User.to_json()
   ```
   But note: multi-line pipe chains are not supported (Pitfall 8).

**Detection:** Any ORM usage that requires more than one let binding inside a case arm hits this. Expect it in every handler.

**Phase mapping:** Should be addressed in the first phase (compiler additions) before building the ORM library itself. If not addressed, every phase will require workarounds.

**Compiler additions needed:** Multi-expression case arms (`do...end` block after `->`), or at minimum, `let` chains in case arm position.

---

### Pitfall 8: Single-Line Pipe Chains Make Query Builder Unusable

**What goes wrong:** The ORM's showcase feature is pipe-chain query building:
```
let users = User |> where("age", ">", 21) |> order_by("name") |> limit(10) |> preload(["posts"]) |> Repo.all()
```
This is a single 100+ character line. The intended readable form:
```
User
|> where("age", ">", 21)
|> order_by("name")
|> limit(10)
|> preload(["posts"])
|> Repo.all()
```
...does not parse because Mesh's parser treats the newline after `User` as a statement terminator, and `|>` at the start of the next line is a syntax error.

**Why it happens:** Mesh parser does not support multi-line pipe continuation. This is documented in STATE.md and encountered repeatedly in Mesher development. The parenthesized workaround exists but requires wrapping the entire chain in parentheses.

**Consequences:**
- The ORM's primary ergonomic advantage (pipe-chain queries) is unreadable on single lines
- Developers fall back to intermediate `let` bindings, making queries verbose:
  ```
  let q = where(User, "age", ">", 21)
  let q = order_by(q, "name")
  let q = limit(q, 10)
  let q = preload(q, ["posts"])
  let users = Repo.all(q)
  ```
  This works but loses the declarative pipe-chain style that makes ORMs ergonomic
- The language's "Elixir-inspired syntax" promise is broken for the exact feature (query building) where pipes shine most

**Prevention:**
1. **Add multi-line pipe continuation to the parser.** If a line ends with `|>`, treat the next line as a continuation. Or: if a line starts with `|>`, treat it as a continuation of the previous expression. This is the highest-impact parser change for ORM ergonomics and for the language generally.

2. **If parser is not extended:** Support parenthesized multi-line pipes. Check if the existing parenthesized workaround works:
   ```
   let users = (User
     |> where("age", ">", 21)
     |> order_by("name")
     |> limit(10)
     |> Repo.all())
   ```
   If this works, document it as the standard ORM query pattern. If not, fix parenthesized expressions to suppress newline-as-terminator inside parens.

3. **Design the query builder API for both styles.** Support both pipe chains and method-chaining via dot syntax:
   ```
   let q = Query.from(User)
   let q = q.where("age", ">", 21)
   let q = q.order_by("name")
   ```
   Dot-syntax method calls are already supported in Mesh and work across lines.

**Detection:** Any query with more than 2 clauses will exceed 80-100 characters on a single line.

**Phase mapping:** Should be addressed in the first phase (compiler additions). The query builder design depends on whether multi-line pipes are available.

**Compiler additions needed:** Multi-line pipe continuation in the parser, OR verified parenthesized workaround.

---

## Moderate Pitfalls

---

### Pitfall 9: No Keyword Arguments -- Verbose Configuration APIs

**What goes wrong:** Ecto's ergonomic API relies heavily on keyword arguments:
```elixir
Repo.all(User, where: [age: {:>, 21}], order_by: :name, limit: 10)
has_many :posts, Post, foreign_key: :author_id
```
Mesh has no keyword arguments. Every configuration option must be a positional parameter or a struct/map. Relationship definitions become:
```
# Positional -- fragile, which param is which?
has_many("posts", Post, "author_id")

# Map -- verbose but clear
has_many("posts", Post, %{"foreign_key" => "author_id"})
```
Neither is ergonomic. Positional arguments are error-prone when there are many optional parameters. Maps lose type safety (string keys, string values).

**Why it happens:** Mesh does not support keyword arguments, named parameters, or optional parameters with defaults. All function parameters are positional and required.

**Consequences:**
- Relationship definitions require remembering parameter order (is it `has_many(name, target, fk)` or `has_many(target, name, fk)`?)
- Configuration-heavy APIs (migration column options, relationship options, query builder options) become walls of positional parameters
- Default values must be handled by providing overloaded functions (different arities) or by accepting Option types and checking for None

**Prevention:**
1. **Use configuration structs instead of keyword arguments.** Define a `HasManyConfig` struct:
   ```
   struct HasManyOpts do
     foreign_key :: Option<String>
     through :: Option<String>
   end
   ```
   But this is verbose for the common case where defaults suffice.

2. **Use convention over configuration.** The ORM infers defaults from naming conventions:
   - `has_many("posts", Post)` infers foreign key as `user_id` (singular of parent table + `_id`)
   - `belongs_to("author", User)` infers foreign key as `author_id` on the current table
   - Only require explicit configuration when conventions do not apply

   This dramatically reduces the need for keyword arguments.

3. **Consider adding keyword arguments to Mesh.** This is a compiler change of moderate scope. Keyword arguments in Mesh could desugar to a Map or struct parameter:
   ```
   has_many("posts", Post, foreign_key: "author_id")
   # desugars to:
   has_many("posts", Post, %{"foreign_key" => "author_id"})
   ```
   This benefits the entire language, not just the ORM.

4. **Use builder pattern for complex configuration.** For migration columns:
   ```
   let col = Column.new("name", "string") |> Column.not_null() |> Column.unique()
   ```
   Each method returns a modified column definition. This works with Mesh's existing pipe operator.

**Detection:** If any ORM function has more than 4 positional parameters, the API is too hard to use correctly.

**Phase mapping:** API design phase. Should be established before building relationship definitions and migration column specs.

**Compiler additions needed:** Keyword arguments (desugaring to Map or struct) would significantly improve ORM ergonomics. If not added, convention-over-configuration minimizes the need.

---

### Pitfall 10: Cross-Module from_json/from_row Resolution Failure

**What goes wrong:** The Mesh type checker has a known limitation where cross-module `Type.from_json()` resolution fails. This was encountered in Mesher development (Phase 88): `EventPayload.from_json(json)` called from a different module than where `EventPayload` is defined fails with "no trait impl providing from_json."

For the ORM, this means: calling `User.from_row(row)` from a different module than where `User` is defined may fail. Since the ORM library and the user's schema definitions are in different modules, this is the default case, not an edge case.

**Why it happens:** The type checker's trait impl resolution for `from_json`/`from_row` is scoped to the module where the struct is defined. Cross-module resolution requires the impl to be visible in the importing module's context, which does not always happen correctly.

**Consequences:**
- ORM operations that hydrate structs from query results fail when called from a different module
- The workaround (thin wrapper functions in the defining module) defeats the purpose of an ORM library
- Every schema struct must export explicit wrapper functions for from_row, defeating the `deriving` automation

**Prevention:**
1. **Fix cross-module from_row/from_json resolution before building the ORM.** This is not optional -- it is a prerequisite. The ORM's core operation (query result -> struct) must work across module boundaries.

2. **The fix is in `infer.rs`:** ensure that `FromRow` and `FromJson` trait impls registered by `deriving(...)` are visible in the importing module's trait resolution context. The existing decision "Trait impls unconditionally exported" (XMOD-05, v1.8) suggests this should work, but the implementation has edge cases.

3. **Test cross-module from_row early.** Write a two-module test where module A defines `struct User end deriving(Row)` and module B calls `User.from_row(map)`. If this fails, fix it before proceeding.

**Detection:** Any `Type.from_row()` call in a module that did not define `Type` triggers this. It will be the first thing that fails when testing the ORM.

**Phase mapping:** Compiler fixes phase (first phase). Blocking for all subsequent ORM development.

**Compiler additions needed:** Fix cross-module `FromRow`/`FromJson` trait resolution edge cases in `infer.rs`.

---

### Pitfall 11: Relationship Metadata Without Runtime Reflection

**What goes wrong:** Relationship definitions like `has_many("posts", Post)` need to be stored as schema metadata and queried at runtime by the preloader. The preloader needs to know: "User has a has_many relationship called 'posts' targeting the Post table with foreign key 'user_id'." In ActiveRecord, this metadata is stored in class instance variables populated by runtime reflection. In Ecto, it is stored in module attributes populated by macros. Mesh has neither.

**Why it happens:** Relationship metadata is inherently cross-schema: User's relationship to Post requires knowledge of both schemas. Without runtime reflection, this metadata must be generated at compile time and stored in a form accessible at runtime. But Mesh has no global registry, no module attributes, and no static variables.

**Consequences:**
- Relationships cannot be "discovered" at runtime -- they must be explicitly passed to every function that needs them
- The preloader needs relationship metadata to generate the correct SQL, but cannot look it up from the struct type alone
- Generic preloading (`Repo.preload(records, ["posts"])`) requires a way to go from the string "posts" to the relationship metadata

**Prevention:**
1. **Generate relationship metadata as a function on the schema module.** `User.__rel__("posts")` returns a `Relationship` struct containing `{kind: "has_many", target_table: "posts", foreign_key: "user_id", target_module: "Post"}`. This function is generated by `deriving(Schema)` when relationship declarations are present.

2. **Relationship declarations are compiler directives, not runtime calls.** They must be part of the struct definition or a companion declaration that the compiler processes:
   ```
   struct User do
     id :: Int
     name :: String
   end deriving(Schema)

   schema User do
     has_many "posts", Post
     belongs_to "organization", Organization
   end
   ```
   The `schema User do...end` block is a new compiler construct that associates relationship metadata with the User struct. The compiler generates `User.__rel__` functions from this.

3. **Pass relationship metadata through the preload call chain.** Instead of runtime lookup, the preloader receives relationship metadata as a parameter. The ORM's `preload` function resolves `"posts"` to relationship metadata at the call site (where the schema is in scope) and passes it down.

4. **Use a schema registry function generated at build time.** The compiler generates a `Schema.lookup(table_name)` function that returns metadata for any schema struct defined with `deriving(Schema)`. This is a switch/case over table name strings, generated at compile time.

**Detection:** If the preloader cannot determine the foreign key or target table for a relationship without explicit parameters at the call site, the metadata propagation is incomplete.

**Phase mapping:** Schema and relationship definition phase.

**Compiler additions needed:** New `schema Struct do...end` block syntax, or extend `deriving(Schema)` to accept relationship declarations inline. Generate `__rel__` metadata functions.

---

### Pitfall 12: Changeset Validation Without Mutable Accumulation

**What goes wrong:** Ecto changesets accumulate validation errors as they flow through a pipeline:
```elixir
changeset
|> validate_required([:name, :email])
|> validate_format(:email, ~r/@/)
|> validate_length(:name, min: 2, max: 100)
```
Each validation function receives a changeset, adds errors if validation fails, and returns the changeset. In Ecto, the changeset struct is immutable (Elixir data is immutable), but new structs are created with updated error lists using `%{changeset | errors: new_errors}`.

In Mesh, structs are also immutable, but Mesh has no struct update syntax (`%{struct | field: value}`). Creating a new struct with one field changed requires reconstructing the entire struct with all fields spelled out.

**Why it happens:** Mesh has no struct update/copy syntax. To change one field of a 10-field struct, you must write out all 10 fields. For a changeset with fields like `{data, changes, errors, valid, action, params}`, each validation step requires reconstructing the full struct.

**Consequences:**
- Each validation function must reconstruct the entire changeset struct to add an error
- Chaining 5 validations means 5 full struct reconstructions, each listing every field
- The code is extremely verbose and error-prone (forgetting to copy a field produces a compile error, but it is tedious)
- The pipe-chain validation pattern becomes impractical

**Prevention:**
1. **Add struct update syntax to Mesh.** Allow `%{changeset | errors: new_errors}` or `Changeset { ...changeset, errors: new_errors }`. This is essential for functional data transformation patterns. The compiler generates code that copies all other fields from the original struct.

2. **If struct update is not added:** Implement changeset as a Map instead of a struct. Maps support `Map.put(changeset, "errors", new_errors)` without reconstructing the entire container. This trades type safety for ergonomics.

3. **Use builder functions that encapsulate the reconstruction.** `Changeset.add_error(changeset, field, message)` internally reconstructs the changeset. The user never writes the full struct literal. The pipeline becomes:
   ```
   changeset
   |> Changeset.validate_required(["name", "email"])
   |> Changeset.validate_length("name", 2, 100)
   ```
   Each function handles the internal struct reconstruction.

4. **Keep the Changeset struct minimal.** Fewer fields = less reconstruction pain. Core fields: `data` (the original struct as a Map), `changes` (Map of changed fields), `errors` (List of error tuples), `valid` (Bool).

**Detection:** If changeset validation functions are more than 10 lines long due to struct reconstruction boilerplate, the approach needs simplification.

**Phase mapping:** Changeset phase. Depends on whether struct update syntax is added to the compiler.

**Compiler additions needed:** Struct update syntax (`{ ...existing, field: new_value }`) is highly recommended. This benefits the entire language, not just changesets.

---

### Pitfall 13: Expression Problem with Query Composition

**What goes wrong:** The query builder supports `where`, `order_by`, `limit`, `offset`, `select`, and `join`. A user wants to add a custom query clause (e.g., `where_between` for date ranges, `where_ilike` for case-insensitive matching, `full_text_search`). Because the query is a closed struct with fixed fields, adding new clause types requires modifying the query builder's source code.

In Ecto, this is solved with macros -- users can write custom query macros that generate SQL fragments. In Diesel, it is solved with Rust's trait system -- users implement traits on their types to extend the query DSL. Mesh has neither macros nor the trait sophistication to make this work at the user level.

**Consequences:**
- Every SQL feature not supported by the query builder requires raw SQL escape hatches
- The raw SQL escape hatch (`Query.raw_where(q, "tsquery @@ to_tsquery($1)", [search])`) bypasses type safety
- Users cannot extend the query builder without modifying the ORM library source
- PostgreSQL-specific features (JSONB operators, array operators, full-text search, CTEs) are either unsupported or require raw SQL

**Prevention:**
1. **Provide a `fragment` function for SQL fragments.** Allow users to inject raw SQL fragments with parameterized values:
   ```
   User |> where_fragment("age BETWEEN $1 AND $2", [min_age, max_age])
   ```
   This is Ecto's `fragment()` approach. It breaks type safety for the fragment but maintains parameterization for values.

2. **Support common PostgreSQL operations natively.** Build `where_in`, `where_not_null`, `where_is_null`, `where_like`, `order_by_desc`, `group_by`, `having` into the query builder. These cover 90% of real-world query needs.

3. **Accept that the query builder will not cover all SQL.** The escape hatch to raw SQL via `Pool.query` is always available. The ORM should make the common case easy, not make every case possible.

4. **Target PostgreSQL only.** Since Mesh targets only PostgreSQL (no database portability goal), the query builder can use PostgreSQL-specific SQL syntax freely. No need for a database-agnostic abstraction layer.

**Detection:** If more than 20% of queries in the Mesher rewrite require raw SQL escape hatches, the query builder's coverage is insufficient.

**Phase mapping:** Query builder phase. The fragment escape hatch should be available from the first query builder version.

---

### Pitfall 14: Map.collect Integer Key Assumption Breaks Query Result Grouping

**What goes wrong:** ORM operations frequently need to group results by string keys (foreign key values for preloading, column values for aggregation). Using `Iter.collect()` to build a `Map<String, List<Record>>` from query results fails silently because `Map.collect` assumes integer keys.

This is the same issue documented in the Mesher v9.0 pitfalls (Pitfall 12 in the original file) but becomes critical for the ORM because:
- Preloading requires grouping child records by foreign key value (string): `group_by(posts, fn(p) -> p.user_id end)` -> `Map<String, List<Post>>`
- Aggregation queries need string-keyed result maps
- The ORM operates on string-keyed data at every layer (text protocol)

**Why it happens:** The `Iter.collect()` codegen path calls `mesh_map_new()` which defaults to `KEY_TYPE_INT`. String keys require `KEY_TYPE_STR` but the collect path does not propagate key type information.

**Consequences:**
- Preloading breaks silently: child records cannot be matched to parent records because map lookups use integer comparison on string pointer values
- Aggregation produces maps with duplicate keys (same string at different addresses treated as different keys)
- This is the ORM's most insidious bug because it produces wrong results without any error

**Prevention:**
1. **Fix Map.collect to propagate key type.** This should be a compiler fix in the collect codegen path: inspect the key type of the iterator element and pass the correct `KEY_TYPE_STR` or `KEY_TYPE_INT` to `mesh_map_new()`.

2. **If not fixed:** Build the grouping function in the ORM using manual `Map.new()` + `Map.put()` in a `for` loop, which correctly tags the map as string-keyed when the first `Map.put` uses a string key.

3. **Test string-keyed grouping early.** Write a test that groups records by a string foreign key and verifies the grouped map has the correct number of entries. This is the canary test for this bug.

**Detection:** Test: insert 3 records with foreign_key "user_1" and 2 records with "user_2". Group by foreign key. The resulting map should have 2 entries (keys "user_1" and "user_2"), not 5 entries.

**Phase mapping:** Must be fixed before the preloading phase. Preloading is impossible without correct string-keyed maps.

**Compiler additions needed:** Fix `Map.collect` key type propagation in the collect codegen path.

---

### Pitfall 15: Newline-as-Terminator Breaks Relationship Declaration Syntax

**What goes wrong:** The schema relationship syntax needs a block-style declaration:
```
schema User do
  has_many "posts", Post
  belongs_to "org", Organization
end
```
But each `has_many`/`belongs_to` line is a function call. If the parser treats newlines as statement terminators, the block body parses each line as an independent statement. The compiler needs to understand that these are declarations within a `schema` block, not standalone function calls.

**Why it happens:** Mesh's parser treats newlines as statement terminators. In a `do...end` block, each line is a separate statement. If `has_many` and `belongs_to` are function calls, they need to return values or have side effects. But schema declarations are metadata, not computations. They need to be processed at compile time, not at runtime.

**Consequences:**
- If `has_many` is a runtime function, it needs a mutable registry to store the metadata -- impossible in Mesh
- If `has_many` is a compiler-processed declaration, it needs new parser support
- The newline terminator means multi-argument declarations that span lines need continuation handling

**Prevention:**
1. **Handle relationship metadata in `deriving(Schema)` with struct annotations** rather than a separate block:
   ```
   struct User do
     id :: Int
     name :: String
     # @has_many "posts", Post
     # @belongs_to "org", Organization
   end deriving(Schema)
   ```
   Use special comments or a new annotation syntax that the compiler processes during `deriving(Schema)` expansion.

2. **Use a companion function that the compiler recognizes:**
   ```
   fn User.__relationships__() do
     [HasMany("posts", "Post", "user_id"), BelongsTo("org", "Organization", "org_id")]
   end
   ```
   The compiler inlines this function and extracts relationship metadata at compile time. This works with existing Mesh syntax -- no parser changes needed.

3. **Encode relationships in the struct itself using phantom fields or a special derive argument:**
   ```
   struct User do
     id :: Int
     name :: String
   end deriving(Schema, has_many("posts", Post), belongs_to("org", Organization))
   ```
   Extend the `deriving(...)` syntax to accept arguments. This requires a parser change to the `deriving(...)` clause but is a targeted change.

4. **Simplest approach: separate schema definition module.** A `UserSchema` module contains functions that return relationship metadata. The ORM reads these at compile time via known function name conventions.

**Detection:** If relationship declarations require runtime mutation or dynamic registration, the approach is incompatible with Mesh.

**Phase mapping:** Schema definition phase (first phase).

**Compiler additions needed:** Extended `deriving(...)` syntax with arguments, OR annotation syntax, OR recognized companion function pattern.

---

## Minor Pitfalls

---

### Pitfall 16: No Atom/Symbol Type for ORM Identifiers

**What goes wrong:** Ecto uses atoms extensively for field names, relationship names, and configuration keys: `:name`, `:has_many`, `:foreign_key`. Mesh has no atom type -- strings are used instead. This means every ORM identifier is a string, which is heap-allocated and compared by value (character-by-character) rather than by identity.

**Prevention:**
1. Accept strings as the identifier type. String comparison in Mesh is efficient for short strings (field names are typically < 20 characters).
2. Use compile-time string constants where possible. The compiler can intern string literals.
3. Do not add an atom type for the ORM alone. The cost/benefit ratio is poor for a single use case.

**Phase mapping:** All phases. Not a blocking issue, just a minor ergonomic difference from Ecto.

---

### Pitfall 17: Transaction Handling in Immutable Context

**What goes wrong:** ORM insert/update operations within a transaction need to pass the transaction connection through every call:
```
Pg.transaction(pool, fn(conn) do
  let user_id = Repo.insert(conn, user)?
  let post = Post { user_id: user_id, title: "Hello" }
  Repo.insert(conn, post)?
  Ok(user_id)
end)
```
Ecto's `Repo.transaction` uses process dictionary (mutable, actor-local state) to implicitly pass the transaction connection. Mesh has no process dictionary or thread-local storage.

**Prevention:**
1. **Explicit connection passing is correct for Mesh.** It is more explicit than Ecto's implicit process dictionary approach and prevents the common Ecto bug of accidentally using a non-transactional connection inside a transaction block.
2. **The existing `Pg.transaction(pool, fn(conn) do ... end)` pattern works.** The ORM's `Repo.transaction` wraps this with ORM-level operations. The callback receives the connection explicitly.
3. **Ensure all Repo functions accept an optional connection parameter** to enable transaction use. `Repo.insert(changeset)` uses pool checkout; `Repo.insert(changeset, conn)` uses the provided transaction connection.

**Phase mapping:** Repo operations phase.

---

### Pitfall 18: UUID Primary Keys vs Integer Primary Keys

**What goes wrong:** Mesher uses UUID primary keys (`gen_random_uuid()`). The ORM assumes integer auto-incrementing primary keys (`SERIAL`). Or vice versa -- the ORM only supports UUIDs and a user wants integer PKs. The primary key type affects: how `Repo.get(User, id)` works, how relationships resolve foreign keys, how the migration generates PK columns, and how INSERT RETURNING parses the returned ID.

**Prevention:**
1. **Support both UUID and Integer primary keys** via schema metadata. The `id` field's Mesh type determines the PK strategy: `id :: Int` -> `SERIAL PRIMARY KEY`, `id :: String` -> `UUID PRIMARY KEY DEFAULT gen_random_uuid()`.
2. **Default to UUID** to match Mesher's existing pattern and PostgreSQL best practices for distributed systems.
3. **The text protocol makes this easier:** both UUID and integer PKs arrive as strings from `deriving(Row)`. The coercion layer handles the difference transparently.

**Phase mapping:** Schema definition phase.

---

### Pitfall 19: TyVar Normalization Edge Cases in Generic Query Results

**What goes wrong:** The query builder returns generic results: `Repo.all(query) :: Result<List<T>, String>` where `T` is the schema struct type. Cross-module type inference for generic return types has known edge cases (TyVar normalization). If the type variable `T` is not correctly resolved when `Repo.all` is called from a different module than where the schema is defined, the compiler may produce incorrect LLVM IR or type errors.

**Prevention:**
1. **Test cross-module generic return types early.** Write a test where module A defines `struct User end deriving(Schema)`, module B calls `Repo.all(User.query())`, and the result is used as `List<User>`.
2. **If TyVar issues arise:** add explicit return type annotations at the call site:
   ```
   let users :: Result<List<User>, String> = Repo.all(query)
   ```
3. **Keep Repo functions monomorphic where possible.** Instead of a generic `Repo.all<T>(query) -> Result<List<T>, String>`, generate per-schema functions: `User.all(query) -> Result<List<User>, String>`.

**Detection:** Type errors or LLVM verification failures when using query results across module boundaries.

**Phase mapping:** Query builder phase.

---

### Pitfall 20: Repo.get vs Repo.get! Error Handling Convention

**What goes wrong:** In Ecto, `Repo.get(User, 42)` returns `nil` when not found (Option), while `Repo.get!(User, 42)` raises an exception. Mesh has no exceptions -- all errors use `Result<T, E>`. The API convention must be clear:
- `Repo.get(User, id)` -> `Result<User, String>` where Err means "not found" or "database error"
- There is no `Repo.get!` equivalent since Mesh has no exceptions

**Prevention:**
1. **Use `Result<Option<User>, String>`** for get operations: `Ok(Some(user))` = found, `Ok(None)` = not found, `Err(msg)` = database error. This distinguishes "not found" from "database error."
2. **Provide both `Repo.get` and `Repo.get_by`:** `get` fetches by primary key, `get_by` fetches by arbitrary column. Both return `Result<Option<T>, String>`.
3. **Do NOT use `panic!`** for "not found" errors. Mesh's let-it-crash philosophy applies to actor supervision, not to expected database query results.

**Phase mapping:** Repo operations phase.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| **Compiler additions (first)** | Single-expression case arms, single-line pipes, cross-module from_row | Fix parser/typeck before building ORM (Pitfalls 7, 8, 10) |
| **Compiler additions (first)** | No struct update syntax | Add `{...struct, field: value}` syntax (Pitfall 12) |
| **Compiler additions (first)** | Map.collect integer key assumption | Fix collect codegen key type propagation (Pitfall 14) |
| **Schema definition** | No macros for DSL | Use `deriving(Schema)` compiler-level generation (Pitfall 1) |
| **Schema definition** | Relationship metadata without reflection | Compiler-generated metadata functions (Pitfall 11) |
| **Schema definition** | Newline terminators in declarations | Use deriving args or companion functions (Pitfall 15) |
| **Query builder** | String column names, no type safety | Compiler-generated column accessors (Pitfall 2) |
| **Query builder** | Pipe chains unreadable | Multi-line pipes or parenthesized workaround (Pitfall 8) |
| **Query builder** | Expression problem, extensibility | Fragment escape hatch, common operators built-in (Pitfall 13) |
| **Relationships & preloading** | N+1 queries | Ecto-style separate-query preloading (Pitfall 3) |
| **Relationships & preloading** | String-keyed grouping broken | Fix Map.collect or manual grouping (Pitfall 14) |
| **Changeset** | No struct update syntax | Builder functions or struct update syntax (Pitfall 12) |
| **Changeset** | Text protocol all-strings coercion | Centralized type coercion in schema metadata (Pitfall 4) |
| **Migrations** | Multi-line SQL strings | Plain SQL files, not Mesh code (Pitfall 5) |
| **Migrations** | Rollback data loss | Forward-only, expand-migrate-contract (Pitfall 6) |
| **Repo operations** | No keyword arguments | Convention over configuration (Pitfall 9) |
| **Repo operations** | Transaction connection passing | Explicit conn parameter (Pitfall 17) |
| **Cross-module usage** | TyVar normalization for generic results | Test early, explicit annotations (Pitfall 19) |

---

## Recommended Compiler Additions (Prioritized)

Based on pitfall analysis, these compiler additions should be implemented BEFORE building the ORM library, in priority order:

| Priority | Addition | Pitfalls Addressed | Scope |
|----------|----------|-------------------|-------|
| **P0** | Fix cross-module from_row/from_json resolution | 10 | typeck fix in infer.rs |
| **P0** | Fix Map.collect string key type propagation | 14 | codegen fix in collect path |
| **P1** | Multi-expression case arms (`do...end` in case) | 7 | parser change |
| **P1** | Multi-line pipe continuation | 8 | parser change |
| **P1** | Struct update syntax (`{...s, field: val}`) | 12 | parser + typeck + codegen |
| **P2** | `deriving(Schema)` with relationship args | 1, 11, 15 | typeck + MIR lowering |
| **P2** | Multi-line string literals | 5 | lexer + parser |
| **P3** | Keyword arguments (optional) | 9 | parser + typeck |

P0 items are bugfixes that block ORM correctness. P1 items are language features that block ORM ergonomics. P2 items enable the ORM-specific compiler features. P3 items are nice-to-have improvements.

---

## Sources

### Primary (HIGH confidence -- direct Mesh source analysis)
- `crates/mesh-typeck/src/infer.rs:2276` -- `valid_derives` array, entry point for new derive implementations
- `crates/mesh-codegen/src/mir/lower.rs:1689-1746` -- `lower_struct_def`, `generate_from_row_struct`, derive dispatch
- `crates/mesh-codegen/src/mir/lower.rs:3914` -- `generate_from_row_struct` implementation, template for `deriving(Schema)`
- `.planning/PROJECT.md:235` -- Single-line pipe chain limitation
- `.planning/PROJECT.md:236` -- Map.collect integer key assumption
- `.planning/phases/88-ingestion-pipeline/88-02-SUMMARY.md:142-143` -- Cross-module from_json resolution failure documented
- `.planning/phases/87.1-issues-encountered/87.1-RESEARCH.md:246-248` -- Row struct all-String fields limitation
- `mesher/storage/queries.mpl` -- 600+ lines of manual Pool.query/Pool.execute demonstrating what the ORM replaces
- `mesher/types/event.mpl:40-60` -- Event struct with all-String fields for deriving(Row) compatibility
- `.planning/STATE.md:44-48` -- Known blockers/concerns for ORM development

### Secondary (MEDIUM confidence -- ORM ecosystem research, multiple sources agree)
- [Ecto.Schema documentation](https://hexdocs.pm/ecto/Ecto.Schema.html) -- Macro-based schema DSL architecture
- [Ecto.Repo preload documentation](https://hexdocs.pm/ecto/Ecto.Repo.html) -- Separate-query preloading strategy
- [Ecto Associations guide](https://hexdocs.pm/ecto/associations.html) -- Relationship definition patterns
- [Diesel ORM documentation](https://diesel.rs/) -- Compile-time type-safe query builder via Rust type system
- [Diesel schema generation](https://docs.rs/diesel) -- table! macro and diesel print-schema approach
- [Haskell Persistent](https://github.com/yesodweb/persistent) -- Template Haskell for schema DSL generation
- [Esqueleto type-safe EDSL](https://hackage.haskell.org/package/esqueleto) -- Type-safe SQL query builder on Persistent
- [William Yao: Type-safe DB libraries comparison](https://williamyaoh.com/posts/2019-12-14-typesafe-db-libraries.html) -- Haskell ORM ecosystem analysis
- [Drizzle ORM approach](https://betterstack.com/community/guides/scaling-nodejs/drizzle-vs-prisma/) -- Code-first schema without codegen
- [SQL injection in ORMs](https://snyk.io/blog/sql-injection-orm-vulnerabilities/) -- Column name injection, parameterization gaps
- [Atlas: Database rollback hard truths](https://atlasgo.io/blog/2024/11/14/the-hard-truth-about-gitops-and-db-rollbacks) -- Forward-only migration philosophy
- [Database migrations: safe strategies](https://vadimkravcenko.com/shorts/database-migrations/) -- Expand-migrate-contract pattern
- [Ecto data mapping and validation](https://hexdocs.pm/ecto/data-mapping-and-validation.html) -- Changeset architecture, casting, validation pipeline
