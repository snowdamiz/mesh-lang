#![cfg(unix)]

#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("cannot locate test executable")
        .parent()
        .expect("test executable has no parent")
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
}

/// Builds tests/e2e/<fixture>.mpl as a project named `name`, whose binary is
/// `<project>/<name>`.
fn build_fixture(fixture: &str, name: &str) -> (tempfile::TempDir, PathBuf, Output) {
    artifacts::ensure_mesh_rt_staticlib();
    let temp = tempfile::tempdir().expect("failed to create temp directory");
    let project = temp.path().join(name);
    std::fs::create_dir_all(&project).expect("failed to create project directory");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/e2e")
        .join(format!("{fixture}.mpl"));
    std::fs::copy(&source, project.join("main.mpl"))
        .unwrap_or_else(|error| panic!("failed to copy {}: {error}", source.display()));
    let output = Command::new(meshc_bin())
        .args(["build", project.to_str().expect("non-UTF-8 project path")])
        .output()
        .expect("failed to invoke meshc");
    (temp, project, output)
}

fn build() -> (tempfile::TempDir, PathBuf, Output) {
    build_fixture("postgres_db_values", "postgres-db-values")
}

#[test]
fn postgres_db_value_public_api_compiles() {
    let (_temp, _project, output) = build();
    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL or the documented local mesh_test PostgreSQL"]
fn postgres_bytea_round_trips_through_public_mesh_api() {
    let (_temp, project, output) = build();
    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("postgres-db-values"))
        .output()
        .expect("failed to execute PostgreSQL DbValue fixture");
    assert!(
        run.status.success(),
        "fixture failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "binary:00ff80\ntext:typed\nnull:null\nlegacy:legacy\ndone\n"
    );
}

/// Every Repo operation (with the Query and Expr builders, changesets,
/// transactions and preloading) against PostgreSQL, in a schema of its own:
/// results, and each operation's failure on a bad table or query.
#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL or the documented local mesh_test PostgreSQL"]
fn postgres_repo_runs_every_repository_operation() {
    let (_temp, project, output) = build_fixture("postgres_repo", "postgres-repo");
    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("postgres-repo"))
        .output()
        .expect("failed to execute the Repo fixture");
    assert!(
        run.status.success(),
        "fixture failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), EXPECTED_REPO_OUTPUT);
}

/// Every Migration function against PostgreSQL, in a schema of its own:
/// what it did to the table, and its failures.
#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL or the documented local mesh_test PostgreSQL"]
fn postgres_migrations_change_the_table_they_name() {
    let (_temp, project, output) = build_fixture("postgres_migration", "postgres-migration");
    assert!(
        output.status.success(),
        "meshc build failed:\n{}",
        artifacts::command_output_text(&output)
    );
    let run = Command::new(project.join("postgres-migration"))
        .output()
        .expect("failed to execute the Migration fixture");
    assert!(
        run.status.success(),
        "{}",
        artifacts::command_output_text(&run)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        EXPECTED_MIGRATION_OUTPUT
    );
}

const EXPECTED_MIGRATION_OUTPUT: &str = "create:ok
create_again:ok
add:ok
add_again:ok
add_plain:ok
add_untyped:failed
rename:ok
rename_missing:failed
drop_column:ok
drop_column_again:ok
columns:id,name,age,email
index:ok
index_unique:ok
index_named:ok
index_no_columns:failed
index_bad_option:failed
indexes:CREATE UNIQUE INDEX idx_people_email ON mesh_migration_e2e.people USING btree (email) \
WHERE (email IS NOT NULL) | CREATE INDEX idx_people_name_age ON mesh_migration_e2e.people USING \
btree (name, age DESC) | CREATE INDEX people_by_age ON mesh_migration_e2e.people USING btree (age)
drop_index:ok
drop_index_again:ok
execute:ok
execute_bad:failed
comment:migrated
drop:ok
drop_again:ok
done
";

const EXPECTED_REPO_OUTPUT: &str = r#"insert:Ada
insert_expr:BOB
insert_duplicate:failed
insert_expr_bad_table:failed
update:4
update_missing_column:failed
update_where:b
update_where_expr:90
update_where_expr_bad:failed
upsert_insert:1
upsert_update:5
upsert_expr:12
upsert_bad_conflict:failed
execute_raw:2
execute_raw_bad:failed
query_raw:Second,Third
query_raw_bad:failed
all:ada,bob,cy,dee
where_op:dee,cy
where_in:ada,cy
where_not_in:bob,dee
where_between:ada,bob,dee
where_null:ada,dee
where_not_null:bob,cy
where_or:ada,bob
where_expr:cy,dee
where_raw:cy
select_limit_offset:bob,cy
select_exprs:-,b,c,-
select_raw:DEE,CY,BOB,ADA
join:First,Second
join_as:Ada,Ada,BOB
group_having:ada
group_by_raw:ada,bob
aggregates:3
where_sub:ada,bob
fragment:cy
all_bad:failed
one:Cy
one_none:failed
one_bad:failed
get:Second
get_none:failed
get_bad:failed
get_by:cy
get_by_none:failed
get_by_bad:failed
count:2
count_bad:failed
exists:true
exists_not:false
exists_bad:failed
pgcrypto:0
exprs:less=3 third=3 json={"a": 1} contains=t id=00000000-0000-0000-0000-000000000001 day=2026-01-02 found=t ranked=t salt=29 hash=$1$abcdefgh$IQtUouv7y7Q9dRWkQEPCc. value=fallback
upsert_qualified:50
upsert_labelled:failed
select_star:A
query_as:ada=50,error:bob has no decodable score,cy=90,dee=12
query_as_bad:failed
preload_posts:[{"author_id":"ada","comments":[{"body":"nice","id":"1","post_id":"1"},{"body":"great","id":"2","post_id":"1"}],"id":"1","title":"First","views":"11"},{"author_id":"ada","comments":[],"id":"2","title":"Second","views":"31"}],[{"author_id":"bob","comments":[{"body":"ok","id":"3","post_id":"3"}],"id":"3","title":"Third","views":"20"}],[]
preload_profile:{"author_id":"ada","bio":"mathematician","id":"1"},null,null
preload_unknown:failed
preload_unknown_nested:failed
preload_author:{"handle":"ada","name":"Ada","nickname":"A","profile":{"author_id":"ada","bio":"mathematician","id":"1"},"score":"50"},{"handle":"ada","name":"Ada","nickname":"A","profile":{"author_id":"ada","bio":"mathematician","id":"1"},"score":"50"},{"handle":"bob","name":"BOB","nickname":"b","profile":null,"score":"2"}
preload_empty:
insert_changeset:Fay
insert_changeset_invalid:can't be blank
insert_changeset_duplicate:has already been taken
update_changeset:Faye
update_changeset_duplicate:has already been taken
update_changeset_invalid:can't be blank
transaction:committed
transaction_rollback:rolled back
transaction_titles:1
delete:ok
delete_none:failed
delete_where:1
delete_where_bad:failed
delete_where_returning_unfiltered:failed
delete_where_returning:great
delete_where_returning_bad:failed
done
"#;

/// The PostgreSQL schema helpers against a database, in a schema of their
/// own: an extension, a range-partitioned table, GIN indexes (one opclass
/// schema-qualified), daily partitions ahead and an old one listed and
/// dropped, and each helper's refusals.
#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL or the documented local mesh_test PostgreSQL"]
fn postgres_schema_helpers_create_list_and_drop() {
    let (_temp, project, output) = build_fixture("postgres_schema", "postgres-schema");
    assert!(
        output.status.success(),
        "meshc build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(project.join("postgres-schema"))
        .output()
        .expect("failed to execute the schema fixture");
    assert!(
        run.status.success(),
        "fixture failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), EXPECTED_SCHEMA_OUTPUT);
}

const EXPECTED_SCHEMA_OUTPUT: &str = r#"extension:ok
extension_again:ok
extension_empty:error:Pg.create_extension: extension name must not be empty
extension_unknown:failed
table:ok
table_again:ok
table_empty_name:error:Pg.create_range_partitioned_table: table name must not be empty
table_no_partition_column:error:Pg.create_range_partitioned_table: partition column must not be empty
table_bad_column:error:Pg.create_range_partitioned_table: invalid column definition `:date`
table_blank_column:error:Pg.create_range_partitioned_table: invalid column definition ` `
table_only_constraints:error:Pg.create_range_partitioned_table: at least one column is required
table_partition_column_missing:error:Pg.create_range_partitioned_table: partition column `at` is missing from `t`
gin_trgm:ok
gin_qualified:ok
gin_empty_table:error:Pg.create_gin_index: table name must not be empty
gin_empty_index:error:Pg.create_gin_index: index name must not be empty
gin_empty_column:error:Pg.create_gin_index: column name must not be empty
gin_empty_opclass:error:Pg.create_gin_index: opclass must not be empty
gin_empty_segment:error:Pg.create_gin_index: identifier `pg_catalog..ops` contains an empty segment
ahead:ok
ahead_none:ok
ahead_negative:error:Pg.create_daily_partitions_ahead: days must be non-negative, got -1
ahead_empty_parent:error:Pg.create_daily_partitions_ahead: parent table must not be empty
ahead_missing_parent:failed
before:events_20000101
before_none:
before_negative:error:Pg.list_daily_partitions_before: max_days must be non-negative, got -1
drop:ok
drop_empty:error:Pg.drop_partition: partition name must not be empty
before_dropped:
partitions:3
done
"#;
