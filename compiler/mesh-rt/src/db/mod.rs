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
