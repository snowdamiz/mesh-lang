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
preload_posts:[{"author_id":"ada","comments":[{"body":"nice","id":"1","post_id":"1"},{"body":"great","id":"2","post_id":"1"}],"id":"1","title":"First","views":"11"},{"author_id":"ada","comments":[],"id":"2","title":"Second","views":"31"}],[{"author_id":"bob","comments":[{"body":"ok","id":"3","post_id":"3"}],"id":"3","title":"Third","views":"20"}],[]
preload_profile:{"author_id":"ada","bio":"mathematician","id":"1"},null,null
preload_unknown:failed
preload_unknown_nested:failed
preload_author:{"handle":"ada","name":"Ada","nickname":"","profile":{"author_id":"ada","bio":"mathematician","id":"1"},"score":"4"},{"handle":"ada","name":"Ada","nickname":"","profile":{"author_id":"ada","bio":"mathematician","id":"1"},"score":"4"},{"handle":"bob","name":"BOB","nickname":"b","profile":null,"score":"2"}
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
