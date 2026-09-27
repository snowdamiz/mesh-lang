//! Query builder runtime module for the Mesh runtime.
//!
//! Provides an immutable, pipe-composable Query struct that accumulates
//! SQL clauses. Each builder function allocates a new Query via
//! `mesh_gc_alloc_actor`, copies the previous state, and modifies the
//! relevant slots. The Query object is never mutated in place.
//!
//! ## Query object layout (14 slots, 112 bytes)
//!
//! | Slot | Offset | Name            | Type                   |
//! |------|--------|-----------------|------------------------|
//! |  0   |   0    | source          | *mut u8 (MeshString)   |
//! |  1   |   8    | select_fields   | *mut u8 (List<String>) |
//! |  2   |  16    | where_clauses   | *mut u8 (List<String>) |
//! |  3   |  24    | where_params    | *mut u8 (List<String>) |
//! |  4   |  32    | order_fields    | *mut u8 (List<String>) |
//! |  5   |  40    | limit_val       | i64 (-1 = no limit)    |
//! |  6   |  48    | offset_val      | i64 (-1 = no offset)   |
//! |  7   |  56    | join_clauses    | *mut u8 (List<String>) |
//! |  8   |  64    | group_fields    | *mut u8 (List<String>) |
//! |  9   |  72    | having_clauses  | *mut u8 (List<String>) |
//! | 10   |  80    | having_params   | *mut u8 (List<String>) |
//! | 11   |  88    | fragment_parts  | *mut u8 (List<String>) |
//! | 12   |  96    | fragment_params | *mut u8 (List<String>) |
//! | 13   | 104    | subqueries      | *mut u8 (List<Query>)  |

use super::quote_ident;
use crate::collections::list::{
    list_strings, mesh_list_append, mesh_list_get, mesh_list_length, mesh_list_new,
};
use crate::gc::mesh_gc_alloc_actor;
use crate::string::mesh_str;
use crate::string::text_of;

// ── Constants ────────────────────────────────────────────────────────

const QUERY_SLOTS: usize = 14;
const QUERY_SIZE: usize = QUERY_SLOTS * 8; // 112 bytes

// Slot indices
const SLOT_SOURCE: usize = 0;
const SLOT_SELECT: usize = 1;
const SLOT_WHERE_CLAUSES: usize = 2;
const SLOT_WHERE_PARAMS: usize = 3;
const SLOT_ORDER: usize = 4;
const SLOT_LIMIT: usize = 5;
const SLOT_OFFSET: usize = 6;
const SLOT_JOIN: usize = 7;
const SLOT_GROUP: usize = 8;
const SLOT_HAVING_CLAUSES: usize = 9;
const SLOT_HAVING_PARAMS: usize = 10;
const SLOT_FRAGMENT_PARTS: usize = 11;
const SLOT_FRAGMENT_PARAMS: usize = 12;
const SLOT_SUBQUERIES: usize = 13;

// ── Slot access helpers ──────────────────────────────────────────────

unsafe fn query_get(q: *mut u8, slot: usize) -> *mut u8 {
    *(q.add(slot * 8) as *mut *mut u8)
}

unsafe fn query_set(q: *mut u8, slot: usize, val: *mut u8) {
    *(q.add(slot * 8) as *mut *mut u8) = val;
}

unsafe fn query_get_int(q: *mut u8, slot: usize) -> i64 {
    *(q.add(slot * 8) as *mut i64)
}

unsafe fn query_set_int(q: *mut u8, slot: usize, val: i64) {
    *(q.add(slot * 8) as *mut i64) = val;
}

/// A Query's clauses, read out of its slots for the SQL builder (repo.rs).
#[derive(Clone, Debug)]
pub(crate) struct QueryParts {
    pub(crate) source: String,
    pub(crate) select: Vec<String>,
    pub(crate) where_clauses: Vec<String>,
    pub(crate) where_params: Vec<String>,
    pub(crate) order: Vec<String>,
    /// -1: no LIMIT.
    pub(crate) limit: i64,
    /// -1: no OFFSET.
    pub(crate) offset: i64,
    pub(crate) joins: Vec<String>,
    pub(crate) group: Vec<String>,
    pub(crate) having: Vec<String>,
    pub(crate) having_params: Vec<String>,
    pub(crate) fragments: Vec<String>,
    pub(crate) fragment_params: Vec<String>,
    /// The queries of its `where_sub` clauses, in clause order.
    pub(crate) subqueries: Vec<QueryParts>,
}

/// The clauses of the Query `q`.
pub(crate) unsafe fn query_parts(q: *mut u8) -> QueryParts {
    let strings = |slot| list_strings(query_get(q, slot));
    QueryParts {
        source: text_of(query_get(q, SLOT_SOURCE)).to_string(),
        select: strings(SLOT_SELECT),
        where_clauses: strings(SLOT_WHERE_CLAUSES),
        where_params: strings(SLOT_WHERE_PARAMS),
        order: strings(SLOT_ORDER),
        limit: query_get_int(q, SLOT_LIMIT),
        offset: query_get_int(q, SLOT_OFFSET),
        joins: strings(SLOT_JOIN),
        group: strings(SLOT_GROUP),
        having: strings(SLOT_HAVING_CLAUSES),
        having_params: strings(SLOT_HAVING_PARAMS),
        fragments: strings(SLOT_FRAGMENT_PARTS),
        fragment_params: strings(SLOT_FRAGMENT_PARAMS),
        subqueries: {
            let subs = query_get(q, SLOT_SUBQUERIES);
            (0..mesh_list_length(subs))
                .map(|i| query_parts(mesh_list_get(subs, i) as *mut u8))
                .collect()
        },
    }
}

// ── Atom-to-SQL mapping ──────────────────────────────────────────────

// An atom none of these know is a Mesh panic, as DateTime.add's unknown
// unit is: the type checker lets any atom through.

fn atom_to_sql_op(atom: &str) -> &'static str {
    match atom {
        "eq" => "=",
        "neq" => "!=",
        "lt" => "<",
        "gt" => ">",
        "lte" => "<=",
        "gte" => ">=",
        "like" => "LIKE",
        "ilike" => "ILIKE",
        _ => crate::panic::raise(format_args!(
            "Query.where_op: unknown operator :{atom}; the operators are :eq, :neq, :lt, \
             :lte, :gt, :gte, :like and :ilike"
        )),
    }
}

fn atom_to_direction(atom: &str) -> &'static str {
    match atom {
        "asc" => "ASC",
        "desc" => "DESC",
        _ => crate::panic::raise(format_args!(
            "Query.order_by: unknown direction :{atom}; the directions are :asc and :desc"
        )),
    }
}

/// `builder` names the Query function, for the panic.
fn atom_to_join_type(atom: &str, builder: &str) -> &'static str {
    match atom {
        "inner" => "INNER",
        "left" => "LEFT",
        "right" => "RIGHT",
        _ => crate::panic::raise(format_args!(
            "Query.{builder}: unknown join kind :{atom}; the kinds are :inner, :left and :right"
        )),
    }
}

// ── Query allocation helpers ─────────────────────────────────────────

/// Allocate a fresh Query with all pointer slots set to empty lists
/// and integer slots set to -1.
unsafe fn alloc_query() -> *mut u8 {
    let q = mesh_gc_alloc_actor(QUERY_SIZE as u64, 8);
    std::ptr::write_bytes(q, 0, QUERY_SIZE);
    // Initialize list slots to empty lists
    let empty = mesh_list_new();
    query_set(q, SLOT_SELECT, empty);
    query_set(q, SLOT_WHERE_CLAUSES, mesh_list_new());
    query_set(q, SLOT_WHERE_PARAMS, mesh_list_new());
    query_set(q, SLOT_ORDER, mesh_list_new());
    query_set(q, SLOT_JOIN, mesh_list_new());
    query_set(q, SLOT_GROUP, mesh_list_new());
    query_set(q, SLOT_HAVING_CLAUSES, mesh_list_new());
    query_set(q, SLOT_HAVING_PARAMS, mesh_list_new());
    query_set(q, SLOT_FRAGMENT_PARTS, mesh_list_new());
    query_set(q, SLOT_FRAGMENT_PARAMS, mesh_list_new());
    query_set(q, SLOT_SUBQUERIES, mesh_list_new());
    // Integer slots: -1 means "not set"
    query_set_int(q, SLOT_LIMIT, -1);
    query_set_int(q, SLOT_OFFSET, -1);
    q
}

/// A Mesh panic unless the raw SQL `sql` takes as many parameters as
/// `params` holds: the builder numbers the values in order, and one over
/// would go to a later clause's placeholder.
unsafe fn check_parameters(builder: &str, sql: &str, params: *mut u8) {
    let (_, takes) = crate::db::repo::renumber_placeholders(sql, 1);
    let given = mesh_list_length(params);
    if takes as i64 != given {
        crate::panic::raise(format_args!(
            "Query.{builder}: `{sql}` takes {takes} parameter(s) but was given {given}"
        ));
    }
}

/// Clone a Query: allocate a new one and copy all data from source.
unsafe fn clone_query(src: *mut u8) -> *mut u8 {
    let dst = mesh_gc_alloc_actor(QUERY_SIZE as u64, 8);
    std::ptr::copy_nonoverlapping(src, dst, QUERY_SIZE);
    dst
}

/// The elements of the Mesh list `list`.
unsafe fn items(list: *mut u8) -> impl Iterator<Item = u64> {
    (0..mesh_list_length(list)).map(move |i| mesh_list_get(list, i))
}

/// `values` appended to the list in `slot` of `new_q`, a fresh copy.
unsafe fn push(new_q: *mut u8, slot: usize, values: impl IntoIterator<Item = u64>) {
    let list = values
        .into_iter()
        .fold(query_get(new_q, slot), |list, value| {
            mesh_list_append(list, value)
        });
    query_set(new_q, slot, list);
}

/// A copy of `q` with `values` appended to the list in `slot`.
unsafe fn appended(q: *mut u8, slot: usize, values: impl IntoIterator<Item = u64>) -> *mut u8 {
    let new_q = clone_query(q);
    push(new_q, slot, values);
    new_q
}

/// A copy of `q` with the text `entry` appended to the list in `slot`.
unsafe fn with_entry(q: *mut u8, slot: usize, entry: &str) -> *mut u8 {
    appended(q, slot, [mesh_str(entry) as u64])
}

/// A copy of `q` with the WHERE clause `clause` and the values it binds.
unsafe fn with_where(q: *mut u8, clause: &str, values: impl IntoIterator<Item = u64>) -> *mut u8 {
    let new_q = clone_query(q);
    push(new_q, SLOT_WHERE_CLAUSES, [mesh_str(clause) as u64]);
    push(new_q, SLOT_WHERE_PARAMS, values);
    new_q
}

/// A copy of `q` selecting the aggregate `function("field")` too.
unsafe fn with_aggregate(q: *mut u8, function: &str, field: *mut u8) -> *mut u8 {
    let aggregate = format!("RAW:{function}({})", quote_ident(text_of(field)));
    with_entry(q, SLOT_SELECT, &aggregate)
}

// ── Extern C builder functions ───────────────────────────────────────

/// Create a new Query from a table name string.
///
/// `Query.from("users")` -> opaque Query pointer
#[no_mangle]
pub extern "C" fn mesh_query_from(table: *mut u8) -> *mut u8 {
    unsafe {
        let q = alloc_query();
        query_set(q, SLOT_SOURCE, table);
        q
    }
}

/// Add an equality WHERE clause: `field = value`.
///
/// `Query.where(q, :name, "Alice")` -> new Query with WHERE name = $N
#[no_mangle]
pub extern "C" fn mesh_query_where(q: *mut u8, field: *mut u8, value: *mut u8) -> *mut u8 {
    unsafe { with_where(q, &format!("{} =", text_of(field)), [value as u64]) }
}

/// Add an operator WHERE clause: `field op value`.
///
/// `Query.where_op(q, :age, :gt, "21")` -> new Query with WHERE age > $N
#[no_mangle]
pub extern "C-unwind" fn mesh_query_where_op(
    q: *mut u8,
    field: *mut u8,
    op: *mut u8,
    value: *mut u8,
) -> *mut u8 {
    unsafe {
        let clause = format!("{} {}", text_of(field), atom_to_sql_op(text_of(op)));
        with_where(q, &clause, [value as u64])
    }
}

/// Add a WHERE IN clause: `field IN (values...)`.
///
/// `Query.where_in(q, :status, ["active", "pending"])` -> new Query with WHERE status IN ($N, $M)
#[no_mangle]
pub extern "C" fn mesh_query_where_in(q: *mut u8, field: *mut u8, values: *mut u8) -> *mut u8 {
    unsafe {
        let clause = format!("{} IN:{}", text_of(field), mesh_list_length(values));
        with_where(q, &clause, items(values))
    }
}

/// Add a WHERE NOT IN clause: `field NOT IN (values...)`.
///
/// `Query.where_not_in(q, :status, ["archived", "deleted"])` -> new Query with WHERE status NOT IN ($N, $M)
#[no_mangle]
pub extern "C" fn mesh_query_where_not_in(q: *mut u8, field: *mut u8, values: *mut u8) -> *mut u8 {
    unsafe {
        let clause = format!("{} NOT_IN:{}", text_of(field), mesh_list_length(values));
        with_where(q, &clause, items(values))
    }
}

/// Add a WHERE BETWEEN clause: `field BETWEEN low AND high`.
///
/// `Query.where_between(q, :age, "18", "65")` -> new Query with WHERE age BETWEEN $N AND $M
#[no_mangle]
pub extern "C" fn mesh_query_where_between(
    q: *mut u8,
    field: *mut u8,
    low: *mut u8,
    high: *mut u8,
) -> *mut u8 {
    unsafe {
        let clause = format!("{} BETWEEN", text_of(field));
        with_where(q, &clause, [low as u64, high as u64])
    }
}

/// Add a WHERE OR clause: `(field1 = $N OR field2 = $M ...)`.
///
/// `Query.where_or(q, [:status, :level], ["active", "error"])` -> new Query with WHERE (status = $N OR level = $M)
///
/// Each field takes the value at its place: lists of different lengths are
/// a Mesh panic.
#[no_mangle]
pub extern "C-unwind" fn mesh_query_where_or(
    q: *mut u8,
    fields: *mut u8,
    values: *mut u8,
) -> *mut u8 {
    unsafe {
        let field_names = list_strings(fields);
        let val_count = mesh_list_length(values);
        if field_names.len() as i64 != val_count {
            crate::panic::raise(format_args!(
                "Query.where_or: {} field(s) but {val_count} value(s)",
                field_names.len()
            ));
        }
        // OR clause encoding: "OR:field1,field2,..."
        with_where(q, &format!("OR:{}", field_names.join(",")), items(values))
    }
}

/// Add a WHERE IS NULL clause.
///
/// `Query.where_null(q, :deleted_at)` -> new Query with WHERE deleted_at IS NULL
#[no_mangle]
pub extern "C" fn mesh_query_where_null(q: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { with_where(q, &format!("{} IS NULL", text_of(field)), []) }
}

/// Add a WHERE IS NOT NULL clause.
///
/// `Query.where_not_null(q, :name)` -> new Query with WHERE name IS NOT NULL
#[no_mangle]
pub extern "C" fn mesh_query_where_not_null(q: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { with_where(q, &format!("{} IS NOT NULL", text_of(field)), []) }
}

/// Add a structured expression-valued WHERE predicate.
///
/// `Query.where_expr(q, Expr.eq(Expr.column("password_hash"), Pg.crypt(...)))`
///   -> new Query with the expression (its JSON) in the WHERE list; the SQL
///   builder renders it, numbering its values where they fall.
#[no_mangle]
pub extern "C" fn mesh_query_where_expr(q: *mut u8, expr: *mut u8) -> *mut u8 {
    unsafe { with_where(q, &format!("EXPR:{}", text_of(expr)), []) }
}

/// Set the SELECT fields for the query.
///
/// `Query.select(q, ["id", "name"])` -> new Query with SELECT id, name
#[no_mangle]
pub extern "C" fn mesh_query_select(q: *mut u8, fields: *mut u8) -> *mut u8 {
    unsafe {
        let new_q = clone_query(q);
        query_set(new_q, SLOT_SELECT, fields);
        new_q
    }
}

/// A copy of `q` selecting each of `exprs` (their JSON) too.
unsafe fn with_select_exprs(q: *mut u8, exprs: *mut u8) -> *mut u8 {
    let entries = list_strings(exprs)
        .iter()
        .map(|expr| mesh_str(&format!("EXPR:{expr}")) as u64)
        .collect::<Vec<_>>();
    appended(q, SLOT_SELECT, entries)
}

/// Append a single structured expression-valued SELECT item.
#[no_mangle]
pub extern "C" fn mesh_query_select_expr(q: *mut u8, expr: *mut u8) -> *mut u8 {
    unsafe { with_select_exprs(q, mesh_list_append(mesh_list_new(), expr as u64)) }
}

/// Append structured expression-valued SELECT items.
///
/// `Query.select_exprs(q, [Expr.alias(Expr.coalesce([...]), "label")])`
///   -> new Query with portable expression SELECT items and ordered params.
#[no_mangle]
pub extern "C" fn mesh_query_select_exprs(q: *mut u8, exprs: *mut u8) -> *mut u8 {
    unsafe { with_select_exprs(q, exprs) }
}

/// Add an ORDER BY clause.
///
/// `Query.order_by(q, :name, :asc)` -> new Query with ORDER BY name ASC
#[no_mangle]
pub extern "C-unwind" fn mesh_query_order_by(
    q: *mut u8,
    field: *mut u8,
    direction: *mut u8,
) -> *mut u8 {
    unsafe {
        let order = format!(
            "{} {}",
            text_of(field),
            atom_to_direction(text_of(direction))
        );
        with_entry(q, SLOT_ORDER, &order)
    }
}

/// Add a raw ORDER BY expression (no quoting).
///
/// `Query.order_by_raw(q, "random()")` -> new Query with ORDER BY random()
#[no_mangle]
pub extern "C" fn mesh_query_order_by_raw(q: *mut u8, expression: *mut u8) -> *mut u8 {
    unsafe { with_entry(q, SLOT_ORDER, &format!("RAW:{}", text_of(expression))) }
}

/// Set the LIMIT for the query.
///
/// `Query.limit(q, 10)` -> new Query with LIMIT 10
#[no_mangle]
pub extern "C" fn mesh_query_limit(q: *mut u8, n: i64) -> *mut u8 {
    unsafe {
        let new_q = clone_query(q);
        query_set_int(new_q, SLOT_LIMIT, n);
        new_q
    }
}

/// Set the OFFSET for the query.
///
/// `Query.offset(q, 20)` -> new Query with OFFSET 20
#[no_mangle]
pub extern "C" fn mesh_query_offset(q: *mut u8, n: i64) -> *mut u8 {
    unsafe {
        let new_q = clone_query(q);
        query_set_int(new_q, SLOT_OFFSET, n);
        new_q
    }
}

/// Add a JOIN clause.
///
/// `Query.join(q, :inner, "posts", "users.id = posts.user_id")` -> new Query with INNER JOIN
#[no_mangle]
pub extern "C-unwind" fn mesh_query_join(
    q: *mut u8,
    join_type: *mut u8,
    table: *mut u8,
    on_clause: *mut u8,
) -> *mut u8 {
    unsafe {
        let join = format!(
            "{}:{}:{}",
            atom_to_join_type(text_of(join_type), "join"),
            text_of(table),
            text_of(on_clause)
        );
        with_entry(q, SLOT_JOIN, &join)
    }
}

/// Add a JOIN clause with table alias.
///
/// `Query.join_as(q, :inner, "projects", "p", "p.id = issues.project_id")` -> INNER JOIN projects p ON ...
#[no_mangle]
pub extern "C-unwind" fn mesh_query_join_as(
    q: *mut u8,
    join_type: *mut u8,
    table: *mut u8,
    alias: *mut u8,
    on_clause: *mut u8,
) -> *mut u8 {
    unsafe {
        let join = format!(
            "ALIAS:{}:{}:{}:{}",
            atom_to_join_type(text_of(join_type), "join_as"),
            text_of(table),
            text_of(alias),
            text_of(on_clause)
        );
        with_entry(q, SLOT_JOIN, &join)
    }
}

/// Add a GROUP BY field.
///
/// `Query.group_by(q, :category)` -> new Query with GROUP BY category
#[no_mangle]
pub extern "C" fn mesh_query_group_by(q: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { appended(q, SLOT_GROUP, [field as u64]) }
}

/// Add a raw GROUP BY expression (no quoting).
///
/// `Query.group_by_raw(q, "date_trunc('hour', received_at)")` -> new Query with raw GROUP BY
#[no_mangle]
pub extern "C" fn mesh_query_group_by_raw(q: *mut u8, expression: *mut u8) -> *mut u8 {
    unsafe { with_entry(q, SLOT_GROUP, &format!("RAW:{}", text_of(expression))) }
}

/// Add a HAVING clause.
///
/// `Query.having(q, "count(*) >", "5")` -> new Query with HAVING count(*) > $N
#[no_mangle]
pub extern "C" fn mesh_query_having(q: *mut u8, clause: *mut u8, value: *mut u8) -> *mut u8 {
    unsafe {
        let new_q = clone_query(q);
        push(new_q, SLOT_HAVING_CLAUSES, [clause as u64]);
        push(new_q, SLOT_HAVING_PARAMS, [value as u64]);
        new_q
    }
}

/// Set SELECT fields using raw SQL expressions (no quoting/escaping).
///
/// `Query.select_raw(q, ["count(*)::text AS count", "level"])` -> new Query with raw SELECT expressions
///
/// Each expression is stored with a "RAW:" prefix so the SQL builder emits it verbatim.
/// Can be mixed with Query.select -- normal fields get quoted, RAW: fields don't.
#[no_mangle]
pub extern "C" fn mesh_query_select_raw(q: *mut u8, expressions: *mut u8) -> *mut u8 {
    unsafe {
        let entries = list_strings(expressions)
            .iter()
            .map(|expr| mesh_str(&format!("RAW:{expr}")) as u64)
            .collect::<Vec<_>>();
        appended(q, SLOT_SELECT, entries)
    }
}

/// Add a raw SQL WHERE clause with optional parameter binding.
///
/// `Query.where_raw(q, "expires_at > now()", [])` -> new Query with raw WHERE clause
/// `Query.where_raw(q, "status IN (?, ?)", ["active", "pending"])` -> with param binding
///
/// The clause is stored with a "RAW:" prefix. `?` placeholders in the clause are
/// replaced with the next sequential `$N` by the SQL builder. Parameters are appended
/// to the where_params list; there must be as many as the placeholders take.
#[no_mangle]
pub extern "C-unwind" fn mesh_query_where_raw(
    q: *mut u8,
    clause: *mut u8,
    params: *mut u8,
) -> *mut u8 {
    unsafe {
        let clause_str = text_of(clause);
        check_parameters("where_raw", clause_str, params);
        with_where(q, &format!("RAW:{clause_str}"), items(params))
    }
}

// ── Aggregate SELECT functions ───────────────────────────────────────

/// Add SELECT count(*) to the query.
///
/// `Query.select_count(q)` -> new Query with count(*) in SELECT
#[no_mangle]
pub extern "C" fn mesh_query_select_count(q: *mut u8) -> *mut u8 {
    unsafe { with_entry(q, SLOT_SELECT, "RAW:count(*)") }
}

/// Add SELECT count("field") to the query.
///
/// `Query.select_count_field(q, :assignee_id)` -> new Query with count("assignee_id") in SELECT
#[no_mangle]
pub extern "C" fn mesh_query_select_count_field(q: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { with_aggregate(q, "count", field) }
}

/// Add SELECT sum("field") to the query.
///
/// `Query.select_sum(q, :amount)` -> new Query with sum("amount") in SELECT
#[no_mangle]
pub extern "C" fn mesh_query_select_sum(q: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { with_aggregate(q, "sum", field) }
}

/// Add SELECT avg("field") to the query.
///
/// `Query.select_avg(q, :price)` -> new Query with avg("price") in SELECT
#[no_mangle]
pub extern "C" fn mesh_query_select_avg(q: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { with_aggregate(q, "avg", field) }
}

/// Add SELECT min("field") to the query.
///
/// `Query.select_min(q, :created_at)` -> new Query with min("created_at") in SELECT
#[no_mangle]
pub extern "C" fn mesh_query_select_min(q: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { with_aggregate(q, "min", field) }
}

/// Add SELECT max("field") to the query.
///
/// `Query.select_max(q, :created_at)` -> new Query with max("created_at") in SELECT
#[no_mangle]
pub extern "C" fn mesh_query_select_max(q: *mut u8, field: *mut u8) -> *mut u8 {
    unsafe { with_aggregate(q, "max", field) }
}

/// Add a WHERE IN subquery clause.
///
/// `Query.where_sub(q, :field, sub_query)` -> new Query with WHERE field IN (SELECT ...)
///
/// The clause names the field; the sub_query itself goes on the Query's
/// subquery list, and the SQL builder renders it whole (every clause it has)
/// with its parameters numbered where it falls.
#[no_mangle]
pub extern "C" fn mesh_query_where_sub(q: *mut u8, field: *mut u8, sub_query: *mut u8) -> *mut u8 {
    unsafe {
        let new_q = with_where(q, &format!("SUB:{}", text_of(field)), []);
        push(new_q, SLOT_SUBQUERIES, [sub_query as u64]);
        new_q
    }
}

/// Add a raw SQL fragment.
///
/// `Query.fragment(q, "WHERE custom_fn($1)", params)` -> new Query with raw fragment
/// (`params` as many as its placeholders take)
#[no_mangle]
pub extern "C-unwind" fn mesh_query_fragment(q: *mut u8, sql: *mut u8, params: *mut u8) -> *mut u8 {
    unsafe {
        check_parameters("fragment", text_of(sql), params);
        let new_q = appended(q, SLOT_FRAGMENT_PARTS, [sql as u64]);
        push(new_q, SLOT_FRAGMENT_PARAMS, items(params));
        new_q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> *mut u8 {
        mesh_str(s) as *mut u8
    }

    /// The message of the Mesh panic `build` raises.
    fn panic_of(build: impl FnOnce() -> *mut u8) -> String {
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(build))
            .expect_err("the builder panics");
        payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default()
    }

    /// An operator, direction or join kind the builders do not know is a
    /// Mesh panic naming the ones they do (it read as `=`, ASC or INNER).
    #[test]
    fn an_unknown_atom_is_a_panic() {
        crate::gc::mesh_rt_init();
        let q = mesh_query_from(text("t"));
        assert_eq!(
            panic_of(|| mesh_query_where_op(q, text("a"), text("greater"), text("1"))),
            "Mesh panic: Query.where_op: unknown operator :greater; the operators are \
             :eq, :neq, :lt, :lte, :gt, :gte, :like and :ilike"
        );
        assert_eq!(
            panic_of(|| mesh_query_order_by(q, text("a"), text("up"))),
            "Mesh panic: Query.order_by: unknown direction :up; the directions are :asc and :desc"
        );
        assert_eq!(
            panic_of(|| mesh_query_join(q, text("outer"), text("u"), text("u.id = t.id"))),
            "Mesh panic: Query.join: unknown join kind :outer; the kinds are :inner, :left \
             and :right"
        );
        assert_eq!(
            panic_of(|| mesh_query_join_as(
                q,
                text("full"),
                text("u"),
                text("x"),
                text("x.id = t.id")
            )),
            "Mesh panic: Query.join_as: unknown join kind :full; the kinds are :inner, :left \
             and :right"
        );
    }

    /// `where_or` pairs each field with a value: a value short left a
    /// placeholder without one, a value over shifted every later clause's.
    #[test]
    fn where_or_needs_a_value_for_each_field() {
        use crate::collections::list::string_list;
        crate::gc::mesh_rt_init();
        let q = mesh_query_from(text("t"));
        assert_eq!(
            panic_of(|| mesh_query_where_or(q, string_list(&["a", "b"]), string_list(&["1"]))),
            "Mesh panic: Query.where_or: 2 field(s) but 1 value(s)"
        );
        assert_eq!(
            panic_of(|| mesh_query_where_or(q, string_list(&["a"]), string_list(&["1", "2"]))),
            "Mesh panic: Query.where_or: 1 field(s) but 2 value(s)"
        );
    }

    /// Raw SQL takes as many values as its placeholders number: a value over
    /// was handed to a later clause's placeholder.
    #[test]
    fn raw_sql_takes_a_value_for_each_placeholder() {
        use crate::collections::list::string_list;
        crate::gc::mesh_rt_init();
        let q = mesh_query_from(text("t"));
        assert_eq!(
            panic_of(|| mesh_query_where_raw(
                q,
                text("a = ? AND b <> '?'"),
                string_list(&["1", "2"])
            )),
            "Mesh panic: Query.where_raw: `a = ? AND b <> '?'` takes 1 parameter(s) but was \
             given 2"
        );
        assert_eq!(
            panic_of(|| mesh_query_fragment(q, text("LIMIT $2"), string_list(&["1"]))),
            "Mesh panic: Query.fragment: `LIMIT $2` takes 2 parameter(s) but was given 1"
        );
    }
}
