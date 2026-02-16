# Phase 100: Relationships + Preloading - Research

**Researched:** 2026-02-16
**Domain:** Mesh compiler codegen (enhanced relationship metadata) + Rust runtime (Repo.preload batch loading)
**Confidence:** HIGH

## Summary

Phase 100 enhances the existing relationship declaration infrastructure from Phase 96 and adds a batch preloading system to the Repo module. The relationship declaration syntax (`belongs_to :user, User`, `has_many :posts, Post`, `has_one :profile, Profile`) already works -- it is parsed, stored in the AST, and encoded in `__relationships__()` as `"kind:name:target"` strings. What is missing is (a) richer relationship metadata that includes the foreign key column and the target table name, and (b) the `Repo.preload` runtime function that uses this metadata to execute batch `WHERE foreign_key IN (...)` queries and group results by foreign key.

The implementation divides into two areas:

**Area 1 -- Enhanced relationship metadata (Plan 100-01):** Extend `generate_schema_metadata()` in `lower.rs` to emit richer relationship strings. Currently `"has_many:posts:Post"` tells us the kind, association name, and target struct name, but NOT the foreign key or target table. The preloader needs: (a) the foreign key column name (conventionally `{struct_name_lowercase}_id` for belongs_to/has_many, or the same for has_one), and (b) the target table name (the `__table__()` of the target struct). The relationship encoding should be extended to `"kind:name:target:fk:target_table"`. The foreign key is inferred by convention: for `has_many :posts, Post` on User, the FK is `user_id` (lowercased owner struct name + `_id`). For `belongs_to :user, User` on Post, the FK is `user_id` (the field that exists on the current struct). This requires the lowerer to resolve the target struct's table name at compile time (which it can, since both structs must have `deriving(Schema)`). Additionally, a `__relationship_meta__()` function should be generated that returns the full 5-field encoded strings for runtime consumption by the preloader.

**Area 2 -- Repo.preload runtime function (Plan 100-02):** A new `mesh_repo_preload` runtime function takes a pool handle, a list of parent rows (as `List<Map<String,String>>`), and a list of association names to preload. For each association, it: (1) parses the relationship metadata to determine the target table, foreign key, and cardinality, (2) collects all unique parent IDs (primary key values for has_many/has_one, or foreign key values for belongs_to), (3) executes a single `SELECT * FROM "target_table" WHERE "fk" IN ($1, $2, ...)` query, (4) groups the results by foreign key value, and (5) attaches the grouped results to each parent row as a new key in the Map. For nested preloading (`"posts.comments"`), the process recurses: after loading posts, it preloads comments on the loaded posts. The result is a `List<Map<String,String>>` where each row has additional keys for the preloaded associations (e.g., a `posts` key containing a JSON-encoded list of post maps, or a nested List pointer).

**Primary recommendation:** Implement in two plans: Plan 100-01 extends the compiler's relationship metadata to include FK and target table, plus adds a `__relationship_meta__` function. Plan 100-02 implements `Repo.preload` as a runtime function that parses metadata, executes batch IN queries, groups results, and supports nested preloading.

## Standard Stack

### Core

| Crate | Location | Purpose | Relevance |
|-------|----------|---------|-----------|
| mesh-codegen | crates/mesh-codegen | MIR lowering (enhanced relationship metadata) | Extend `generate_schema_metadata()` with FK + target table info |
| mesh-typeck | crates/mesh-typeck | Register new schema/repo functions | `__relationship_meta__()`, `Repo.preload` type signatures |
| mesh-rt | crates/mesh-rt | Runtime preloading implementation | New `mesh_repo_preload` function in `repo.rs` |

### Supporting

| Library | Version | Purpose | When Used |
|---------|---------|---------|-----------|
| inkwell | 0.8.0 (LLVM 21.1) | LLVM IR generation | Declaring new `mesh_repo_preload` intrinsic |
| insta | 1.46 | Snapshot testing | Verifying enhanced relationship metadata output |

## Architecture Patterns

### Pattern 1: Relationship Metadata Encoding (Established in Phase 96, Extended Here)

**What:** `generate_schema_metadata()` in `lower.rs` (line 4559) produces `__relationships__()` returning `List<String>` with entries like `"has_many:posts:Post"`. This phase adds a `__relationship_meta__()` function returning the full 5-field encoding.

**Current encoding (Phase 96):**
```
"kind:name:target"
```
- kind: "belongs_to", "has_many", "has_one"
- name: association name (e.g., "posts", "user", "profile")
- target: target struct name (e.g., "Post", "User", "Profile")

**Enhanced encoding (Phase 100):**
```
"kind:name:target:fk:target_table"
```
- kind: relationship cardinality
- name: association name
- target: target struct name
- fk: foreign key column name (inferred by convention)
- target_table: the table name of the target struct (resolved at compile time)

**Foreign key inference convention:**
- `has_many :posts, Post` on `User` -> FK is `user_id` (owner struct lowercased + `_id`)
- `has_one :profile, Profile` on `User` -> FK is `user_id` (owner struct lowercased + `_id`)
- `belongs_to :user, User` on `Post` -> FK is `user_id` (association name + `_id`)

This matches the standard Rails/Ecto convention where the FK column lives on the "many" side.

**Target table resolution:** At compile time, the MIR lowerer has access to `generate_schema_metadata()` calls for ALL structs in the module. For cross-module references, the target struct's table name can be resolved by: (a) looking up the generated `{Target}____table__` function's body (a StringLit), or (b) applying the same naive pluralization rule (lowercased + "s") used for the owner struct. Approach (b) is simpler and sufficient since custom table names are rare and the convention is consistent.

### Pattern 2: Batch Preloading (Ecto-Style Separate Queries)

**What:** Instead of JOINing associated records (which duplicates parent data), preloading uses separate queries -- one per association level. This is the established pattern from Ecto and ActiveRecord.

**Algorithm for `Repo.preload(pool, rows, [:posts])`:**

1. Parse the relationship metadata for the parent struct (from `__relationship_meta__()`)
2. Find the `:posts` association -> `"has_many:posts:Post:user_id:posts"`
3. Extract all parent primary key values: `parent_ids = rows.map(|r| r["id"])`
4. Remove duplicates from parent_ids
5. Execute: `SELECT * FROM "posts" WHERE "user_id" IN ($1, $2, ..., $N)`
6. Group results by `user_id`: `grouped = {user_id_1: [post1, post2], user_id_2: [post3], ...}`
7. For each parent row, attach: `row["posts"] = grouped[row["id"]]` (or empty list if no matches)
8. Return the enriched rows

**For belongs_to (reverse direction):**
- `belongs_to :user, User` on Post with FK `user_id`
- Extract all FK values from child rows: `fk_values = rows.map(|r| r["user_id"])`
- Execute: `SELECT * FROM "users" WHERE "id" IN ($1, $2, ..., $N)`
- Group by `id` (the PK of the target)
- Attach: `row["user"] = grouped[row["user_id"]]` (single map, not list)

**For has_one (same as has_many but single result):**
- Same as has_many but each parent gets at most one associated record

### Pattern 3: Nested Preloading

**What:** `Repo.preload(pool, users, [:posts, "posts.comments"])` loads posts for users, then comments for those posts.

**Algorithm:**
1. Parse association list. Split into levels:
   - Level 0 (top-level atoms): `:posts` -> direct association on parent
   - Level 1+ (dot-separated strings): `"posts.comments"` -> nested: load comments on posts
2. Execute level 0 preloads first (load posts onto users)
3. For each nested association `"posts.comments"`:
   - Extract the intermediate collection: all posts from all users
   - Look up `comments` association on the Post struct's relationship metadata
   - Execute batch preload of comments onto posts
   - The posts (already attached to users) now have comments attached
4. Return the fully enriched parent rows

**Nested association syntax:**
- Atoms (`:posts`) -> direct association
- Strings (`"posts.comments"`) -> nested dot-separated path
- This matches the success criteria: `Repo.preload(pool, users, [:posts, "posts.comments"])`

### Pattern 4: Preloaded Data Structure

**What:** Preloaded associations are stored as additional keys in the parent's Map<String, String>.

**Challenge:** The parent row is `Map<String, String>` where values are strings. But a preloaded has_many association is a `List<Map<String, String>>` -- not a string.

**Options:**
1. **Store as JSON string:** Serialize the list of maps to a JSON string and store under the association key. Access via `JSON.parse(row["posts"])`. Simple but loses type information.
2. **Store as Ptr in the map value:** Maps store `u64` values. A `*mut u8` pointer to a List fits in a `u64`. The map value for "posts" would be a List pointer, not a MeshString pointer. The caller must know to treat it as a List, not a String.
3. **Return a separate PreloadResult structure:** A new opaque struct holding the original rows plus a preload map. More complex but type-safe.

**Recommendation: Option 2 (Ptr in map value).** The existing Map<String, String> stores values as `u64` regardless of actual type. A `*mut u8` pointing to a List is a valid `u64` value. The Mesh code accesses preloaded data via `Map.get(row, "posts")` which returns a Ptr. Since `List<Map>` is already a Ptr at runtime, this works transparently. The type checker cannot enforce the distinction (it sees Ptr for both String and List), but the developer knows that preloaded keys contain Lists (for has_many) or Maps (for belongs_to/has_one).

For unloaded associations, the key simply does not exist in the map. Accessing `Map.get(row, "posts")` when posts were not preloaded returns the map's default (0/null pointer), which would cause a crash if dereferenced. A better approach: `Repo.preload` returns enriched maps, and accessing an association that was not preloaded should produce a clear error.

**Error handling for unloaded associations:** Rather than a runtime check (which would require special infrastructure), the documentation should clearly state: "Always use `Repo.preload` before accessing association keys. Accessing an unpreloaded association key returns null and will cause a runtime error." This matches Ecto's `Ecto.Association.NotLoaded` pattern conceptually, though without the dedicated struct.

**Alternative for clear error messages:** Add a sentinel value for unloaded associations. When `deriving(Schema)` generates metadata, the runtime could populate a special marker string (e.g., `"__NOT_LOADED__"`) in the row's map for each declared association. Then `Repo.preload` replaces these with actual data. If the developer accesses an unloaded association, they get the string `"__NOT_LOADED__"` instead of a null crash. This is simple and provides a clear diagnostic.

### Pattern 5: Relationship Metadata Resolution at Runtime

**What:** `Repo.preload` needs to look up relationship metadata for a given struct/table at runtime. The metadata is available via `StructName.__relationship_meta__()`, but the runtime function receives raw rows (maps), not struct types.

**Challenge:** At runtime, `Repo.preload` receives `List<Map<String,String>>` -- it does not know which struct these rows came from. It needs the relationship metadata to determine FK columns and target tables.

**Solution: Pass relationship metadata explicitly.**

```
Repo.preload(pool, users, [:posts], User.__relationship_meta__())
```

This adds a 4th parameter: the relationship metadata list from the parent struct. The runtime function parses this list to determine how to resolve each association.

**Alternative: Infer from table name.** If rows came from a query on "users", the preloader could look up relationships for the "users" table. But there's no runtime registry mapping table names to relationship metadata. Building such a registry adds complexity.

**Recommendation:** Explicit metadata passing. The 4th parameter is `List<String>` from `__relationship_meta__()`. This is explicit, requires no global registry, and matches the existing pattern where schema functions are called explicitly (e.g., `User.__table__()`, `User.__fields__()`).

### Anti-Patterns to Avoid

- **N+1 queries in preloading:** The entire point of preloading is to avoid N+1. Each association level MUST use a single `WHERE fk IN (...)` query, never N individual queries.
- **JOINing for preloading:** JOINs duplicate parent data and make result parsing complex. Use separate queries per association (Ecto pattern).
- **Modifying parent rows in place:** The Map values are stored via `mesh_map_put` which creates a new map (copy-on-write semantics). Each enriched row is a new map with the association key added.
- **Complex MIR for relationship metadata:** Continue using encoded strings ("kind:name:target:fk:target_table") in a List<String>. Do not attempt to build nested maps or structs in MIR for metadata.
- **Preloading single rows:** `Repo.preload` should work on a List of rows, not a single row. For single-row preloading, wrap in a list: `Repo.preload(pool, [user], [:posts], meta)`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Foreign key inference | Complex AST analysis of both structs | Convention: `{owner_lowercase}_id` | Standard Rails/Ecto convention; simple string formatting |
| IN clause SQL generation | Manual string building in preload | Existing `WHERE "fk" IN:N` encoding + `build_select_sql_from_parts` | Already implemented in Phase 98 repo.rs |
| Result grouping | Custom hash map implementation | Rust `HashMap<String, Vec<*mut u8>>` in runtime | Rust stdlib HashMap is efficient and safe |
| Batch ID extraction | Manual map iteration | Existing `list_to_strings` + `map_to_columns_and_values` helpers | Already available in repo.rs |
| GC-safe allocation | Manual malloc for result maps | `mesh_map_put` / `mesh_list_append` | Existing GC-traced collection operations |

**Key insight:** The preloading runtime function is a composition of existing primitives: `mesh_pool_query` for SQL execution, `mesh_map_get`/`mesh_map_put` for row access, `mesh_list_*` for list operations, and the existing IN clause SQL generation from `build_select_sql_from_parts`.

## Common Pitfalls

### Pitfall 1: Foreign Key Convention Mismatch

**What goes wrong:** The inferred FK `user_id` does not match the actual column name in the database (e.g., the column is `author_id`).
**Why it happens:** The convention `{owner_lowercase}_id` is naive and does not handle custom FK names.
**How to avoid:** For Phase 100, use the convention as default. If a future phase needs custom FK names, extend the relationship declaration syntax: `has_many :posts, Post, foreign_key: :author_id`. The parser already supports contextual identifiers in struct bodies. For now, document the convention clearly.
**Warning signs:** "Column user_id does not exist" errors when preloading associations where the FK has a non-standard name.

### Pitfall 2: Target Table Resolution Across Modules

**What goes wrong:** `has_many :posts, Post` on User requires knowing Post's table name at compile time. If User and Post are in different modules, the MIR lowerer may not have Post's schema metadata available when generating User's relationship metadata.
**Why it happens:** Multi-module compilation processes modules in dependency order. If User's module is compiled before Post's module, Post's `__table__()` function body is not yet available.
**How to avoid:** Use the same naive pluralization rule for FK inference: `Post` -> `posts` (lowercased + "s"). This matches the convention from Phase 96-04. If the target struct has a custom table name (`table "blog_posts"`), this inference breaks. However, the custom table name is only available after the target module is compiled. For cross-module cases, the naive rule is the safest default.
**Warning signs:** Incorrect target table name in relationship metadata when target struct uses custom `table` option.

### Pitfall 3: Empty IN Clause

**What goes wrong:** If no parent rows exist (empty list), the preloader builds `WHERE fk IN ()` which is invalid SQL.
**Why it happens:** The IN clause with zero values is a PostgreSQL syntax error.
**How to avoid:** Before building the IN query, check if the parent ID set is empty. If empty, skip the query entirely and return the rows unchanged (with empty lists for has_many associations).
**Warning signs:** PostgreSQL error "syntax error at or near `)` " when preloading on an empty list.

### Pitfall 4: Duplicate Parent IDs in IN Clause

**What goes wrong:** If multiple parent rows have the same ID (shouldn't happen in practice but could with join results), the IN clause includes duplicates, wasting bandwidth.
**Why it happens:** Naive ID extraction from parent rows does not deduplicate.
**How to avoid:** Collect parent IDs into a HashSet before building the IN clause. This deduplicates automatically.
**Warning signs:** Unnecessarily large IN clauses; no functional error but performance degradation.

### Pitfall 5: Nested Preloading Order Dependency

**What goes wrong:** `Repo.preload(pool, users, ["posts.comments", :posts])` tries to load comments on posts before posts are loaded.
**Why it happens:** The association list is processed in order. If the nested association comes before the parent, the intermediate data does not exist yet.
**How to avoid:** Sort associations by nesting depth. Process all level-0 associations first, then level-1, then level-2, etc. Parse dot-separated paths to determine depth.
**Warning signs:** Null pointer when trying to access intermediate association results.

### Pitfall 6: Preloaded Ptr vs String Type Confusion

**What goes wrong:** Developer treats a preloaded has_many value (a List pointer) as a String, or vice versa.
**Why it happens:** Map values are opaque `u64` at runtime. The type checker sees `Ptr` for both String and List.
**How to avoid:** Document clearly: "has_many preloads produce List values, belongs_to/has_one produce Map values." At the type checker level, `Repo.preload` returns `Ptr` (the same `List<Map>` it received, enriched). The developer must know the structure.
**Warning signs:** Crash when calling `String.length()` on a preloaded List value, or `List.get()` on a preloaded String value.

### Pitfall 7: Map Key Type for Association Data

**What goes wrong:** String-keyed maps use `mesh_map_new_typed(1)` with string comparison. Storing a List pointer as a value is fine (values are `u64`), but the key must be a valid MeshString pointer.
**Why it happens:** Map entries are `[key: u64, value: u64]`. For string-keyed maps, the key is a `*mut MeshString`. The value can be any u64 (string pointer, list pointer, integer, etc.).
**How to avoid:** Use `rust_str_to_mesh("posts")` as the key, and the List pointer as the value. The Map does not care about the value type -- it stores raw u64. Reading the value later via `Map.get(row, "posts")` returns the List pointer as a u64 which Mesh sees as a Ptr.
**Warning signs:** None -- this should work correctly. The potential issue is only at the Mesh source level where the developer must cast correctly.

## Code Examples

### Enhanced Relationship Metadata Generation

```rust
// In generate_schema_metadata(), extend the relationship encoding:

// Current (Phase 96): "kind:name:target"
// Extended (Phase 100): "kind:name:target:fk:target_table"

// ── __relationship_meta__() ──────────────────────────────────────
// Returns List<String> with full relationship metadata.
let meta_elements: Vec<MirExpr> = relationships
    .iter()
    .filter_map(|rel| {
        let kind = rel.kind_text()?;
        let assoc = rel.assoc_name()?;
        let target = rel.target_type()?;

        // Infer foreign key by convention
        let fk = match kind.as_str() {
            "belongs_to" => format!("{}_id", assoc),          // belongs_to :user -> user_id
            "has_many" | "has_one" => format!("{}_id", name.to_lowercase()), // has_many on User -> user_id
            _ => return None,
        };

        // Infer target table by convention (naive pluralization)
        let target_table = format!("{}s", target.to_lowercase());

        Some(MirExpr::StringLit(
            format!("{}:{}:{}:{}:{}", kind, assoc, target, fk, target_table),
            MirType::String,
        ))
    })
    .collect();

let meta_fn_name = format!("{}____relationship_meta__", name);
self.functions.push(MirFunction {
    name: meta_fn_name.clone(),
    params: vec![],
    return_type: MirType::Ptr,
    body: MirExpr::ListLit {
        elements: meta_elements,
        ty: MirType::Ptr,
    },
    is_closure_fn: false,
    captures: vec![],
    has_tail_calls: false,
});
self.known_functions.insert(
    meta_fn_name,
    MirType::FnPtr(vec![], Box::new(MirType::Ptr)),
);
```

### Type Checker Registration for __relationship_meta__

```rust
// In infer.rs, inside the Schema derive block:

// __relationship_meta__ :: () -> List<String>
let meta_fn_name = format!("{}.__relationship_meta__", name);
env.insert(meta_fn_name, Scheme::mono(Ty::fun(vec![], Ty::list(Ty::string()))));
```

### Repo.preload Type Signature

```rust
// In infer.rs, Repo module:

// Repo.preload(PoolHandle, Ptr, Ptr, Ptr) -> Ptr
// (pool, rows: List<Map>, associations: List<String/Atom>, relationship_meta: List<String>) -> Result<List<Map>, String>
repo_mod.insert("preload".to_string(), Scheme::mono(Ty::fun(
    vec![pool_t.clone(), ptr_t.clone(), ptr_t.clone(), ptr_t.clone()],
    ptr_t.clone(),
)));
```

### Repo.preload Runtime Implementation

```rust
// In crates/mesh-rt/src/db/repo.rs:

/// Batch preload associated records for a list of parent rows.
///
/// `Repo.preload(pool, rows, associations, relationship_meta)`
///   -> `Result<List<Map<String,String>>, String>`
///
/// For each association in the list:
/// 1. Parse relationship metadata to find FK, target table, cardinality
/// 2. Collect unique parent IDs
/// 3. Execute: SELECT * FROM "target_table" WHERE "fk" IN ($1, $2, ...)
/// 4. Group results by FK value
/// 5. Attach grouped results to each parent row under the association key
#[no_mangle]
pub extern "C" fn mesh_repo_preload(
    pool: u64,
    rows: *mut u8,
    associations: *mut u8,
    rel_meta: *mut u8,
) -> *mut u8 {
    unsafe {
        let row_count = mesh_list_length(rows);
        if row_count == 0 {
            return ok_result(rows); // nothing to preload
        }

        // Parse relationship metadata into lookup map
        let meta_strings = list_to_strings(rel_meta);
        let rel_map = parse_relationship_meta(&meta_strings);

        // Parse association names (may be atoms or dot-separated strings)
        let assoc_names = list_to_strings(associations);

        // Sort by depth: atoms (depth 0) first, then "a.b" (depth 1), etc.
        let mut sorted_assocs: Vec<(usize, String)> = assoc_names
            .iter()
            .map(|a| (a.matches('.').count(), a.clone()))
            .collect();
        sorted_assocs.sort_by_key(|(depth, _)| *depth);

        // Working copy: enrich rows progressively
        let mut current_rows = rows;

        for (_depth, assoc_path) in &sorted_assocs {
            if assoc_path.contains('.') {
                // Nested preload: "posts.comments"
                // Find the intermediate collection and preload on it
                current_rows = preload_nested(
                    pool, current_rows, assoc_path, &rel_map,
                )?;
            } else {
                // Direct preload: "posts"
                current_rows = preload_direct(
                    pool, current_rows, assoc_path, &rel_map,
                )?;
            }
        }

        ok_result(current_rows)
    }
}

/// Parse "kind:name:target:fk:target_table" strings into a lookup structure.
struct RelMeta {
    kind: String,
    name: String,
    target: String,
    fk: String,
    target_table: String,
}

fn parse_relationship_meta(meta_strings: &[String]) -> HashMap<String, RelMeta> {
    let mut map = HashMap::new();
    for entry in meta_strings {
        let parts: Vec<&str> = entry.splitn(5, ':').collect();
        if parts.len() == 5 {
            map.insert(parts[1].to_string(), RelMeta {
                kind: parts[0].to_string(),
                name: parts[1].to_string(),
                target: parts[2].to_string(),
                fk: parts[3].to_string(),
                target_table: parts[4].to_string(),
            });
        }
    }
    map
}
```

### Direct Preload Implementation

```rust
/// Preload a single direct association on a list of rows.
unsafe fn preload_direct(
    pool: u64,
    rows: *mut u8,
    assoc_name: &str,
    rel_map: &HashMap<String, RelMeta>,
) -> Result<*mut u8, *mut u8> {
    let meta = rel_map.get(assoc_name)
        .ok_or_else(|| err_result(&format!("unknown association: {}", assoc_name)))?;

    let row_count = mesh_list_length(rows);

    // Determine which column to extract from parent rows for the IN clause
    let (parent_key, target_match_key) = match meta.kind.as_str() {
        "has_many" | "has_one" => {
            // Parent PK -> target FK: collect parent "id" values,
            // query target WHERE fk IN (...), group by fk
            ("id".to_string(), meta.fk.clone())
        }
        "belongs_to" => {
            // Parent FK -> target PK: collect parent FK values,
            // query target WHERE id IN (...), group by id
            (meta.fk.clone(), "id".to_string())
        }
        _ => return Err(err_result(&format!("unknown relationship kind: {}", meta.kind))),
    };

    // 1. Collect unique parent values for the IN clause
    let parent_key_mesh = rust_str_to_mesh(&parent_key);
    let mut id_set: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for i in 0..row_count {
        let row = mesh_list_get(rows, i) as *mut u8;
        let val = mesh_map_get(row, parent_key_mesh as u64);
        if val != 0 {
            let s = mesh_str_ref(val as *mut u8).to_string();
            if seen.insert(s.clone()) {
                id_set.push(s);
            }
        }
    }

    if id_set.is_empty() {
        // No IDs to query -- attach empty lists/nulls and return
        return Ok(attach_empty_association(rows, assoc_name, &meta.kind));
    }

    // 2. Build and execute the IN query
    let in_count = id_set.len();
    let where_clause = format!("{} IN:{}", target_match_key, in_count);
    let (sql, params) = build_select_sql_from_parts(
        &meta.target_table,
        &[],                           // SELECT *
        &[where_clause],               // WHERE fk IN:N
        &id_set,                       // parameter values
        &[], -1, -1, &[], &[], &[], &[], &[], &[],
    );

    let sql_ptr = rust_str_to_mesh(&sql) as *const MeshString;
    let params_ptr = strings_to_mesh_list(&params);
    let result = mesh_pool_query(pool, sql_ptr, params_ptr);

    let r = &*(result as *const MeshResult);
    if r.tag != 0 {
        return Err(result);
    }
    let result_rows = r.value;

    // 3. Group results by the match key
    let match_key_mesh = rust_str_to_mesh(&target_match_key);
    let result_count = mesh_list_length(result_rows);
    let mut grouped: HashMap<String, Vec<*mut u8>> = HashMap::new();
    for i in 0..result_count {
        let row = mesh_list_get(result_rows, i) as *mut u8;
        let key_val = mesh_map_get(row, match_key_mesh as u64);
        if key_val != 0 {
            let key_str = mesh_str_ref(key_val as *mut u8).to_string();
            grouped.entry(key_str).or_default().push(row);
        }
    }

    // 4. Attach results to each parent row
    let assoc_key_mesh = rust_str_to_mesh(assoc_name);
    let mut enriched = mesh_list_new();
    for i in 0..row_count {
        let row = mesh_list_get(rows, i) as *mut u8;
        let parent_val = mesh_map_get(row, parent_key_mesh as u64);
        let parent_str = if parent_val != 0 {
            mesh_str_ref(parent_val as *mut u8).to_string()
        } else {
            String::new()
        };

        let assoc_data = match meta.kind.as_str() {
            "has_many" => {
                // Build a List of associated rows
                let mut list = mesh_list_new();
                if let Some(matches) = grouped.get(&parent_str) {
                    for &m in matches {
                        list = mesh_list_append(list, m as u64);
                    }
                }
                list as *mut u8
            }
            "has_one" => {
                // Single associated row or null
                grouped.get(&parent_str)
                    .and_then(|v| v.first())
                    .map(|&m| m)
                    .unwrap_or(std::ptr::null_mut())
            }
            "belongs_to" => {
                // Single associated row or null
                grouped.get(&parent_str)
                    .and_then(|v| v.first())
                    .map(|&m| m)
                    .unwrap_or(std::ptr::null_mut())
            }
            _ => std::ptr::null_mut(),
        };

        // Add association to the row's map
        let new_row = mesh_map_put(row, assoc_key_mesh as u64, assoc_data as u64);
        enriched = mesh_list_append(enriched, new_row as u64);
    }

    Ok(enriched)
}
```

### Nested Preloading

```rust
/// Preload a nested association path like "posts.comments"
unsafe fn preload_nested(
    pool: u64,
    rows: *mut u8,
    assoc_path: &str,
    rel_map: &HashMap<String, RelMeta>,
) -> Result<*mut u8, *mut u8> {
    let parts: Vec<&str> = assoc_path.splitn(2, '.').collect();
    if parts.len() != 2 {
        return Err(err_result(&format!("invalid nested association: {}", assoc_path)));
    }
    let parent_assoc = parts[0];
    let child_assoc = parts[1];

    // The parent association must already be loaded
    // Collect all intermediate rows from the parent association
    let row_count = mesh_list_length(rows);
    let parent_key_mesh = rust_str_to_mesh(parent_assoc);
    let mut intermediate_rows = mesh_list_new();
    for i in 0..row_count {
        let row = mesh_list_get(rows, i) as *mut u8;
        let assoc_val = mesh_map_get(row, parent_key_mesh as u64);
        if assoc_val != 0 {
            let assoc_list = assoc_val as *mut u8;
            // Flatten: if has_many, iterate the list; if belongs_to/has_one, single row
            let sub_count = mesh_list_length(assoc_list);
            for j in 0..sub_count {
                let sub_row = mesh_list_get(assoc_list, j);
                intermediate_rows = mesh_list_append(intermediate_rows, sub_row);
            }
        }
    }

    // Now preload child_assoc on the intermediate rows
    // Need the relationship metadata for the intermediate struct
    // The intermediate struct is the target of parent_assoc
    let parent_meta = rel_map.get(parent_assoc)
        .ok_or_else(|| err_result(&format!("unknown association: {}", parent_assoc)))?;

    // For nested preloading, we need the child struct's relationship metadata.
    // This is passed as part of a secondary metadata parameter or resolved
    // from the target struct's __relationship_meta__() at runtime.
    //
    // Simplest approach: require a flat metadata list that includes ALL
    // relationship metadata for all structs involved in preloading.
    // The preload function receives a merged metadata list.

    // Look up child_assoc in the rel_map (assumes merged metadata)
    let enriched_intermediate = preload_direct(
        pool, intermediate_rows, child_assoc, rel_map,
    )?;

    // Re-attach the enriched intermediate rows back to the parent rows
    // by rebuilding the parent association lists
    // (This is the complex part -- need to re-stitch)
    // ...

    Ok(rows) // simplified -- actual implementation re-stitches
}
```

### E2E Test Examples

```rust
#[test]
fn e2e_relationship_meta_has_many() {
    let output = compile_and_run(r#"
struct User do
  id :: String
  name :: String
  has_many :posts, Post
end deriving(Schema)

struct Post do
  id :: String
  title :: String
  user_id :: String
  belongs_to :user, User
end deriving(Schema)

fn main() do
  let meta = User.__relationship_meta__()
  let m0 = List.get(meta, 0)
  println(m0)
  let post_meta = Post.__relationship_meta__()
  let pm0 = List.get(post_meta, 0)
  println(pm0)
end
"#);
    assert_eq!(output, "has_many:posts:Post:user_id:posts\nbelongs_to:user:User:user_id:users\n");
}
```

## State of the Art

| Old Approach (Phase 96-99) | New Approach (Phase 100) | Impact |
|----------------------------|--------------------------|--------|
| `__relationships__()` returns "kind:name:target" | `__relationship_meta__()` returns "kind:name:target:fk:target_table" | Runtime has all info needed for preloading |
| No preloading -- N+1 manual queries | `Repo.preload(pool, rows, assocs, meta)` batch loads | Single query per association level |
| No nested association loading | Dot-separated paths: "posts.comments" | Multi-level preloading in one call |
| Associations not accessible on rows | Map values hold List/Map pointers for preloaded data | Transparent access via Map.get |

## Open Questions

1. **Nested preloading metadata: single merged list vs multiple lists?**
   - What we know: `Repo.preload(pool, users, [:posts, "posts.comments"], meta)` needs relationship metadata for BOTH User and Post structs.
   - What's unclear: Should `meta` be a single merged list (`User.__relationship_meta__() ++ Post.__relationship_meta__()`) or should the API accept a Map of struct-name -> metadata?
   - Recommendation: Accept a single merged List<String> containing all relationship metadata for all structs involved. The 5-field encoding includes enough context (kind, name, target) for the runtime to disambiguate. However, if two structs both have `:posts` associations, there could be collisions. Since the target struct name differs, the runtime can distinguish by also checking the target_table field. For simplicity, start with a merged list and see if collisions are a practical issue.

2. **`__relationship_meta__()` vs extended `__relationships__()`?**
   - What we know: `__relationships__()` already exists with "kind:name:target" format. Adding FK and target_table extends this.
   - What's unclear: Should we modify the existing `__relationships__()` output (breaking backward compat for Phase 96 e2e tests) or add a new `__relationship_meta__()` function?
   - Recommendation: Add a NEW `__relationship_meta__()` function. Keep `__relationships__()` unchanged for backward compatibility. The new function returns the extended 5-field encoding. The preloader uses `__relationship_meta__()`.

3. **Preloaded data access pattern: Map.get vs dedicated accessor?**
   - What we know: Preloaded data stored as additional Map keys. `Map.get(row, "posts")` returns a List pointer.
   - What's unclear: Should there be a `Repo.get_assoc(row, :posts)` function with better error handling, or is raw Map.get sufficient?
   - Recommendation: Start with raw Map.get for simplicity. If the error experience is poor (null crashes), add `Repo.get_assoc` in a follow-up that returns `Result<Ptr, String>` with a "not preloaded" error message.

4. **has_one cardinality: single Map or List of one?**
   - What we know: has_one means at most one associated record. belongs_to also means at most one.
   - What's unclear: Should has_one preloaded value be a single Map pointer (like belongs_to) or a List containing 0-1 elements?
   - Recommendation: Single Map pointer for both belongs_to and has_one (or null if no match). This is the natural expectation: `user["profile"]` returns the profile map directly, not a list of one. For has_many, return a List.

5. **Primary key assumption in preloading**
   - What we know: The preloader assumes "id" as the primary key for collecting parent IDs.
   - What's unclear: Should it use the struct's `__primary_key__()` for the PK column instead of hardcoded "id"?
   - Recommendation: Yes, use `__primary_key__()`. But this means the metadata or an additional parameter must convey the PK. The simplest approach: extend the metadata format to include the owner's PK, or accept it as a 5th parameter. Alternatively, hardcode "id" for now (matching Phase 96 default) and document the limitation.

6. **Re-stitching nested preloads back to parents**
   - What we know: After preloading comments onto posts, the enriched posts need to be re-associated with their parent users.
   - What's unclear: How to re-stitch without losing the parent-child mapping.
   - Recommendation: Use a positional approach. When collecting intermediate rows for nested preloading, track which parent index each intermediate row came from. After enriching intermediate rows, rebuild the parent association lists using the tracked indices. This adds complexity but is necessary for correct nested preloading.

## Sources

### Primary (HIGH confidence)
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 4559-4589 -- existing `__relationships__()` generation with "kind:name:target" encoding
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 4490-4498 -- `generate_schema_metadata()` signature with `relationships: &[RelationshipDecl]`
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 1898-1925 -- Schema derive processing (relationships, schema opts, timestamps)
- Codebase analysis: `crates/mesh-parser/src/parser/items.rs` lines 296-396 -- relationship declaration parser (belongs_to, has_many, has_one with atom + type)
- Codebase analysis: `crates/mesh-parser/src/ast/item.rs` lines 338-385 -- RelationshipDecl AST accessors (kind_text, assoc_name, target_type)
- Codebase analysis: `crates/mesh-typeck/src/infer.rs` lines 2568-2599 -- Schema function registration (__table__, __fields__, __primary_key__, __relationships__, __field_types__, column accessors)
- Codebase analysis: `crates/mesh-codegen/src/mir/lower.rs` lines 6424-6441 -- Schema metadata function resolution in MIR lowering
- Codebase analysis: `crates/mesh-rt/src/db/repo.rs` lines 90-325 -- Repo SQL builders (query_to_select_sql, build_select_sql_from_parts with IN clause support)
- Codebase analysis: `crates/mesh-rt/src/db/repo.rs` lines 573-580 -- mesh_repo_all pattern (query -> SQL -> Pool.query)
- Codebase analysis: `crates/mesh-rt/src/db/query.rs` lines 213-235 -- mesh_query_where_in implementation (IN:N encoding)
- Codebase analysis: `crates/meshc/tests/e2e.rs` lines 3303-3330 -- existing relationship metadata e2e test
- Codebase analysis: `crates/mesh-codegen/src/codegen/intrinsics.rs` lines 989-1048 -- Repo intrinsic declarations
- Codebase analysis: `crates/mesh-rt/src/db/repo.rs` lines 806-831 -- map_to_columns_and_values internal map access
- Codebase analysis: `crates/mesh-rt/src/io.rs` lines 1-50 -- MeshResult struct (tag/value), alloc_result
- Codebase analysis: `.planning/ROADMAP.md` lines 251-265 -- Phase 100 requirements, success criteria, and plan structure
- Codebase analysis: `.planning/REQUIREMENTS.md` -- COMP-06 and REPO-10 requirement definitions
- Codebase analysis: `.planning/STATE.md` -- accumulated decisions from Phases 96-99

### Secondary (MEDIUM confidence)
- Ecto preloading pattern: Separate queries per association level, WHERE fk IN (...), result grouping by FK -- based on established ORM patterns
- Foreign key naming convention: `{table_singular}_id` -- matches Rails/Ecto/ActiveRecord standard convention

## Metadata

**Confidence breakdown:**
- Relationship metadata extension: HIGH -- Direct extension of existing `generate_schema_metadata()` with the same encoded-string pattern; FK inference is a simple string format
- Repo.preload design: HIGH -- Follows established pattern of runtime functions (extern "C", GC-safe allocation, List/Map composition); the SQL generation reuses existing IN clause support
- Nested preloading: MEDIUM -- The re-stitching of nested results back to parent rows adds algorithmic complexity; the positional tracking approach needs careful implementation
- Cross-module FK resolution: MEDIUM -- Naive pluralization works for the common case but breaks with custom table names; acceptable for Phase 100 scope

**Research date:** 2026-02-16
**Valid until:** 2026-03-16 (compiler internals and runtime are stable, controlled by this project)
