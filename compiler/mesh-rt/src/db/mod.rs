pub mod changeset;
pub mod expr;
pub mod json;
pub mod migration;
pub mod orm;
pub mod pg;
pub mod pg_schema;
pub mod pool;
pub mod query;
pub mod repo;
pub mod row;
pub mod sqlite;

/// A SQL identifier, double-quoted with its quotes doubled (PostgreSQL).
pub(crate) fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A name the ORM writes for a table or column, which may be qualified:
/// `schema.table`, `table.column` or `table.*`, each part quoted but `*`.
pub(crate) fn quote_name(name: &str) -> String {
    name.split('.')
        .map(|part| match part {
            "*" => "*".to_string(),
            _ => quote_ident(part),
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// `sql` executed on the pool without parameters (`Pool.execute`'s
/// result), or `Err(message)` for DDL that could not be built.
pub(crate) fn execute_ddl(pool: u64, sql: Result<String, String>) -> *mut u8 {
    match sql {
        Ok(sql) => pool::mesh_pool_execute(
            pool,
            crate::string::mesh_str(&sql),
            crate::collections::list::mesh_list_new(),
        ),
        Err(message) => crate::io::err_result(&message),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_are_quoted_part_by_part() {
        assert_eq!(super::quote_ident("a\"b.c"), "\"a\"\"b.c\"");
        assert_eq!(super::quote_name("writers"), "\"writers\"");
        assert_eq!(
            super::quote_name("public.writers"),
            "\"public\".\"writers\""
        );
        assert_eq!(super::quote_name("writers.*"), "\"writers\".*");
        assert_eq!(super::quote_name("*"), "*");
    }
}
