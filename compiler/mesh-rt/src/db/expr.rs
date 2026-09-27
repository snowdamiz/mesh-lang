//! Neutral SQL expression builder for the Mesh runtime.
//!
//! This layer models portable database expression nodes without
//! exposing vendor-specific helpers. Expressions serialize to SQL fragments plus
//! ordered parameter lists so Query/Repo can embed them into SELECT, WHERE,
//! UPDATE, and ON CONFLICT clauses.

use super::{quote_ident, quote_name};
use crate::collections::list::{mesh_list_get, mesh_list_length};
use crate::string::mesh_str;
use crate::string::text_of;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum SqlExpr {
    Column(String),
    Value(String),
    Null,
    Call {
        name: String,
        args: Vec<SqlExpr>,
    },
    Binary {
        op: String,
        lhs: Box<SqlExpr>,
        rhs: Box<SqlExpr>,
    },
    Case {
        branches: Vec<(SqlExpr, SqlExpr)>,
        else_expr: Box<SqlExpr>,
    },
    Coalesce(Vec<SqlExpr>),
    Excluded(String),
    Cast {
        expr: Box<SqlExpr>,
        sql_type: String,
    },
    Alias {
        expr: Box<SqlExpr>,
        alias: String,
    },
}

impl SqlExpr {
    /// This expression with its bare column names qualified by `table`, as an
    /// upsert's DO UPDATE needs them: there a bare name is ambiguous between
    /// the table's row and EXCLUDED.
    pub(crate) fn qualified(&self, table: &str) -> SqlExpr {
        let q = |expr: &SqlExpr| expr.qualified(table);
        let boxed = |expr: &SqlExpr| Box::new(expr.qualified(table));
        match self {
            SqlExpr::Column(name) if !name.contains('.') => {
                SqlExpr::Column(format!("{table}.{name}"))
            }
            SqlExpr::Call { name, args } => SqlExpr::Call {
                name: name.clone(),
                args: args.iter().map(q).collect(),
            },
            SqlExpr::Binary { op, lhs, rhs } => SqlExpr::Binary {
                op: op.clone(),
                lhs: boxed(lhs),
                rhs: boxed(rhs),
            },
            SqlExpr::Case {
                branches,
                else_expr,
            } => SqlExpr::Case {
                branches: branches
                    .iter()
                    .map(|(cond, value)| (q(cond), q(value)))
                    .collect(),
                else_expr: boxed(else_expr),
            },
            SqlExpr::Coalesce(exprs) => SqlExpr::Coalesce(exprs.iter().map(q).collect()),
            SqlExpr::Cast { expr, sql_type } => SqlExpr::Cast {
                expr: boxed(expr),
                sql_type: sql_type.clone(),
            },
            SqlExpr::Alias { expr, alias } => SqlExpr::Alias {
                expr: boxed(expr),
                alias: alias.clone(),
            },
            other => other.clone(),
        }
    }
}

/// An `Expr` value: the expression as JSON in a Mesh string, which the GC
/// frees with the last value holding it (a boxed `SqlExpr` was never freed).
fn alloc_expr(expr: SqlExpr) -> *mut u8 {
    let json = serde_json::to_string(&expr).expect("an SqlExpr serializes");
    mesh_str(&json) as *mut u8
}

pub(crate) unsafe fn clone_expr(ptr: *mut u8) -> SqlExpr {
    serde_json::from_str(text_of(ptr)).expect("an Expr value holds an SqlExpr")
}

fn render_function_name(name: &str) -> String {
    name.split('.')
        .map(|segment| segment.replace('"', ""))
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn serialize_expr(expr: &SqlExpr) -> (String, Vec<String>) {
    let mut params = Vec::new();
    let mut next_idx = 1usize;
    let sql = render_expr(expr, &mut params, &mut next_idx);
    (sql, params)
}

pub(crate) fn render_expr(
    expr: &SqlExpr,
    params: &mut Vec<String>,
    next_idx: &mut usize,
) -> String {
    match expr {
        SqlExpr::Column(name) => quote_name(name),
        SqlExpr::Value(value) => {
            let idx = *next_idx;
            *next_idx += 1;
            params.push(value.clone());
            format!("${idx}")
        }
        SqlExpr::Null => "NULL".to_string(),
        SqlExpr::Call { name, args } => {
            let rendered_args = args
                .iter()
                .map(|arg| render_expr(arg, params, next_idx))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", render_function_name(name), rendered_args)
        }
        SqlExpr::Binary { op, lhs, rhs } => format!(
            "({} {} {})",
            render_expr(lhs, params, next_idx),
            op,
            render_expr(rhs, params, next_idx)
        ),
        SqlExpr::Case {
            branches,
            else_expr,
        } => {
            let mut sql = String::from("CASE");
            for (cond, value) in branches {
                sql.push_str(" WHEN ");
                sql.push_str(&render_expr(cond, params, next_idx));
                sql.push_str(" THEN ");
                sql.push_str(&render_expr(value, params, next_idx));
            }
            sql.push_str(" ELSE ");
            sql.push_str(&render_expr(else_expr, params, next_idx));
            sql.push_str(" END");
            sql
        }
        SqlExpr::Coalesce(exprs) => {
            let rendered = exprs
                .iter()
                .map(|arg| render_expr(arg, params, next_idx))
                .collect::<Vec<_>>()
                .join(", ");
            format!("COALESCE({rendered})")
        }
        SqlExpr::Excluded(name) => format!("EXCLUDED.{}", quote_ident(name)),
        SqlExpr::Cast { expr, sql_type } => format!(
            "{}::{}",
            render_expr(expr, params, next_idx),
            sql_type.replace('"', "")
        ),
        SqlExpr::Alias { expr, alias } => format!(
            "{} AS {}",
            render_expr(expr, params, next_idx),
            quote_ident(alias)
        ),
    }
}

unsafe fn expr_list_to_vec(list_ptr: *mut u8) -> Vec<SqlExpr> {
    let len = mesh_list_length(list_ptr);
    let mut exprs = Vec::with_capacity(len as usize);
    for i in 0..len {
        let ptr = mesh_list_get(list_ptr, i) as *mut u8;
        exprs.push(clone_expr(ptr));
    }
    exprs
}

fn binary_expr(op: &str, lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    unsafe {
        alloc_expr(SqlExpr::Binary {
            op: op.to_string(),
            lhs: Box::new(clone_expr(lhs)),
            rhs: Box::new(clone_expr(rhs)),
        })
    }
}

fn cast_expr(expr: *mut u8, sql_type: String) -> *mut u8 {
    unsafe {
        alloc_expr(SqlExpr::Cast {
            expr: Box::new(clone_expr(expr)),
            sql_type,
        })
    }
}

fn call_expr(name: &str, args: Vec<SqlExpr>) -> *mut u8 {
    alloc_expr(SqlExpr::Call {
        name: name.to_string(),
        args,
    })
}

#[no_mangle]
pub extern "C" fn mesh_expr_column(field: *mut u8) -> *mut u8 {
    unsafe { alloc_expr(SqlExpr::Column(text_of(field).to_string())) }
}

#[no_mangle]
pub extern "C" fn mesh_expr_value(value: *mut u8) -> *mut u8 {
    unsafe { alloc_expr(SqlExpr::Value(text_of(value).to_string())) }
}

#[no_mangle]
pub extern "C" fn mesh_expr_null() -> *mut u8 {
    alloc_expr(SqlExpr::Null)
}

#[no_mangle]
pub extern "C" fn mesh_expr_call(name: *mut u8, args: *mut u8) -> *mut u8 {
    unsafe {
        alloc_expr(SqlExpr::Call {
            name: text_of(name).to_string(),
            args: expr_list_to_vec(args),
        })
    }
}

#[no_mangle]
pub extern "C" fn mesh_pg_cast(expr: *mut u8, sql_type: *mut u8) -> *mut u8 {
    unsafe { cast_expr(expr, text_of(sql_type).to_string()) }
}

#[no_mangle]
pub extern "C" fn mesh_pg_jsonb(expr: *mut u8) -> *mut u8 {
    cast_expr(expr, "jsonb".to_string())
}

#[no_mangle]
pub extern "C" fn mesh_pg_int(expr: *mut u8) -> *mut u8 {
    cast_expr(expr, "int".to_string())
}

#[no_mangle]
pub extern "C" fn mesh_pg_text(expr: *mut u8) -> *mut u8 {
    cast_expr(expr, "text".to_string())
}

#[no_mangle]
pub extern "C" fn mesh_pg_uuid(expr: *mut u8) -> *mut u8 {
    cast_expr(expr, "uuid".to_string())
}

#[no_mangle]
pub extern "C" fn mesh_pg_timestamptz(expr: *mut u8) -> *mut u8 {
    cast_expr(expr, "timestamptz".to_string())
}

#[no_mangle]
pub extern "C" fn mesh_pg_gen_salt(algorithm: *mut u8, rounds: i64) -> *mut u8 {
    unsafe {
        call_expr(
            "gen_salt",
            vec![
                SqlExpr::Value(text_of(algorithm).to_string()),
                SqlExpr::Value(rounds.to_string()),
            ],
        )
    }
}

#[no_mangle]
pub extern "C" fn mesh_pg_crypt(password: *mut u8, salt: *mut u8) -> *mut u8 {
    unsafe { call_expr("crypt", vec![clone_expr(password), clone_expr(salt)]) }
}

#[no_mangle]
pub extern "C" fn mesh_pg_to_tsvector(config: *mut u8, expr: *mut u8) -> *mut u8 {
    unsafe {
        call_expr(
            "to_tsvector",
            vec![
                SqlExpr::Cast {
                    expr: Box::new(SqlExpr::Value(text_of(config).to_string())),
                    sql_type: "regconfig".to_string(),
                },
                clone_expr(expr),
            ],
        )
    }
}

#[no_mangle]
pub extern "C" fn mesh_pg_plainto_tsquery(config: *mut u8, expr: *mut u8) -> *mut u8 {
    unsafe {
        call_expr(
            "plainto_tsquery",
            vec![
                SqlExpr::Cast {
                    expr: Box::new(SqlExpr::Value(text_of(config).to_string())),
                    sql_type: "regconfig".to_string(),
                },
                clone_expr(expr),
            ],
        )
    }
}

#[no_mangle]
pub extern "C" fn mesh_pg_ts_rank(vector: *mut u8, query: *mut u8) -> *mut u8 {
    unsafe { call_expr("ts_rank", vec![clone_expr(vector), clone_expr(query)]) }
}

#[no_mangle]
pub extern "C" fn mesh_pg_tsvector_matches(vector: *mut u8, query: *mut u8) -> *mut u8 {
    binary_expr("@@", vector, query)
}

#[no_mangle]
pub extern "C" fn mesh_pg_jsonb_contains(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("@>", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_add(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("+", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_sub(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("-", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_mul(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("*", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_div(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("/", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_eq(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("=", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_neq(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("!=", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_lt(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("<", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_lte(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr("<=", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_gt(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr(">", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_gte(lhs: *mut u8, rhs: *mut u8) -> *mut u8 {
    binary_expr(">=", lhs, rhs)
}

#[no_mangle]
pub extern "C" fn mesh_expr_case(
    conditions: *mut u8,
    results: *mut u8,
    else_expr: *mut u8,
) -> *mut u8 {
    unsafe {
        let conds = expr_list_to_vec(conditions);
        let vals = expr_list_to_vec(results);
        let branch_count = conds.len().min(vals.len());
        let mut branches = Vec::with_capacity(branch_count);
        for idx in 0..branch_count {
            branches.push((conds[idx].clone(), vals[idx].clone()));
        }
        alloc_expr(SqlExpr::Case {
            branches,
            else_expr: Box::new(clone_expr(else_expr)),
        })
    }
}

#[no_mangle]
pub extern "C" fn mesh_expr_coalesce(exprs: *mut u8) -> *mut u8 {
    unsafe { alloc_expr(SqlExpr::Coalesce(expr_list_to_vec(exprs))) }
}

#[no_mangle]
pub extern "C" fn mesh_expr_excluded(field: *mut u8) -> *mut u8 {
    unsafe { alloc_expr(SqlExpr::Excluded(text_of(field).to_string())) }
}

#[no_mangle]
pub extern "C" fn mesh_expr_alias(expr: *mut u8, alias: *mut u8) -> *mut u8 {
    unsafe {
        alloc_expr(SqlExpr::Alias {
            expr: Box::new(clone_expr(expr)),
            alias: text_of(alias).to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_alias_coalesce_uses_local_placeholders() {
        let expr = SqlExpr::Alias {
            expr: Box::new(SqlExpr::Coalesce(vec![
                SqlExpr::Column("nickname".into()),
                SqlExpr::Value("fallback".into()),
            ])),
            alias: "nick".into(),
        };

        let (sql, params) = serialize_expr(&expr);
        assert_eq!(sql, "COALESCE(\"nickname\", $1) AS \"nick\"");
        assert_eq!(params, vec!["fallback"]);
    }

    #[test]
    fn serialize_case_and_excluded_preserves_param_order() {
        let expr = SqlExpr::Case {
            branches: vec![(
                SqlExpr::Binary {
                    op: "=".into(),
                    lhs: Box::new(SqlExpr::Column("status".into())),
                    rhs: Box::new(SqlExpr::Value("resolved".into())),
                },
                SqlExpr::Value("unresolved".into()),
            )],
            else_expr: Box::new(SqlExpr::Excluded("status".into())),
        };

        let (sql, params) = serialize_expr(&expr);
        assert_eq!(
            sql,
            "CASE WHEN (\"status\" = $1) THEN $2 ELSE EXCLUDED.\"status\" END"
        );
        assert_eq!(params, vec!["resolved", "unresolved"]);
    }

    #[test]
    fn serialize_cast_supports_vendor_types_without_quoting() {
        let expr = SqlExpr::Cast {
            expr: Box::new(SqlExpr::Value("42".into())),
            sql_type: "jsonb".into(),
        };

        let (sql, params) = serialize_expr(&expr);
        assert_eq!(sql, "$1::jsonb");
        assert_eq!(params, vec!["42"]);
    }

    #[test]
    fn serialize_pg_crypto_and_fulltext_helpers_keep_param_order() {
        let expr = SqlExpr::Call {
            name: "ts_rank".into(),
            args: vec![
                SqlExpr::Call {
                    name: "to_tsvector".into(),
                    args: vec![
                        SqlExpr::Cast {
                            expr: Box::new(SqlExpr::Value("english".into())),
                            sql_type: "regconfig".into(),
                        },
                        SqlExpr::Call {
                            name: "crypt".into(),
                            args: vec![
                                SqlExpr::Value("secret".into()),
                                SqlExpr::Call {
                                    name: "gen_salt".into(),
                                    args: vec![
                                        SqlExpr::Value("bf".into()),
                                        SqlExpr::Value("12".into()),
                                    ],
                                },
                            ],
                        },
                    ],
                },
                SqlExpr::Call {
                    name: "plainto_tsquery".into(),
                    args: vec![
                        SqlExpr::Cast {
                            expr: Box::new(SqlExpr::Value("english".into())),
                            sql_type: "regconfig".into(),
                        },
                        SqlExpr::Value("secret".into()),
                    ],
                },
            ],
        };

        let (sql, params) = serialize_expr(&expr);
        assert_eq!(
            sql,
            "ts_rank(to_tsvector($1::regconfig, crypt($2, gen_salt($3, $4))), plainto_tsquery($5::regconfig, $6))"
        );
        assert_eq!(
            params,
            vec!["english", "secret", "bf", "12", "english", "secret"]
        );
    }

    #[test]
    fn serialize_jsonb_contains_uses_pg_operator() {
        let expr = SqlExpr::Binary {
            op: "@>".into(),
            lhs: Box::new(SqlExpr::Column("events.tags".into())),
            rhs: Box::new(SqlExpr::Cast {
                expr: Box::new(SqlExpr::Value("{\"env\":\"prod\"}".into())),
                sql_type: "jsonb".into(),
            }),
        };

        let (sql, params) = serialize_expr(&expr);
        assert_eq!(sql, "(\"events\".\"tags\" @> $1::jsonb)");
        assert_eq!(params, vec!["{\"env\":\"prod\"}"]);
    }
}
