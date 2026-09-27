//! `meshc migrate up | down | status` against PostgreSQL. The paths that
//! never connect run anywhere; the lifecycle needs MESH_TEST_DATABASE_URL
//! (the coverage run starts a database for it).

#[path = "support/test_artifacts.rs"]
mod test_artifacts;

use std::path::Path;
use std::process::{Command, Output};

use mesh_rt::db::pg::{native_pg_close, native_pg_connect, native_pg_execute};
use test_artifacts::{command_output_text, ensure_mesh_rt_staticlib, meshc_bin};

fn migrate(project: &Path, args: &[&str], database_url: Option<&str>) -> Output {
    let mut command = Command::new(meshc_bin());
    command.arg("migrate").arg(project).args(args);
    command.env_remove("DATABASE_URL");
    if let Some(url) = database_url {
        command.env("DATABASE_URL", url);
    }
    command.output().expect("meshc runs")
}

fn write_migration(project: &Path, file: &str, up: &str, down: &str) {
    let dir = project.join("migrations");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(file),
        format!(
            "pub fn up(pool :: PoolHandle) -> Int!String do\n  {up}\nend\n\npub fn down(pool :: PoolHandle) -> Int!String do\n  {down}\nend\n"
        ),
    )
    .unwrap();
}

#[test]
fn migrations_need_a_database_url() {
    let project = tempfile::tempdir().unwrap();
    for action in ["up", "down", "status"] {
        let output = migrate(project.path(), &[action], None);
        assert_eq!(output.status.code(), Some(1), "{action}");
        assert!(
            command_output_text(&output).contains("DATABASE_URL environment variable is required"),
            "{action}: {}",
            command_output_text(&output)
        );
    }
}

/// With nothing to run, `up` and `status` say so without connecting (the
/// URL here reaches no database), and files not named `<version>_<name>.mpl`
/// are not migrations.
#[test]
fn a_project_without_migrations_needs_no_database() {
    let unreachable = "postgres://nobody:nothing@127.0.0.1:1/none";
    let project = tempfile::tempdir().unwrap();
    for action in ["up", "status"] {
        let output = migrate(project.path(), &[action], Some(unreachable));
        assert!(output.status.success(), "{}", command_output_text(&output));
        assert!(command_output_text(&output).contains("No migrations directory found"));
    }
    let migrations = project.path().join("migrations");
    std::fs::create_dir_all(&migrations).unwrap();
    std::fs::write(migrations.join("README.md"), "notes").unwrap();
    std::fs::write(migrations.join("draft.mpl"), "").unwrap();
    std::fs::write(migrations.join("next_step.mpl"), "").unwrap();
    for action in ["up", "status"] {
        let output = migrate(project.path(), &[action], Some(unreachable));
        assert!(output.status.success(), "{}", command_output_text(&output));
        assert!(
            command_output_text(&output).contains("No migration files found in migrations/"),
            "{}",
            command_output_text(&output)
        );
    }
}

fn database_url() -> String {
    std::env::var("MESH_TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://mesh_test:mesh_test@localhost:5432/mesh_test?sslmode=disable".to_string()
    })
}

/// The whole lifecycle: pending, applied in version order, nothing left to
/// apply, rolled back one at a time, then nothing left to roll back; a
/// migration that fails or does not compile stops the run with its name.
#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL or the documented local mesh_test PostgreSQL"]
fn migrations_apply_report_and_roll_back() {
    let _tracking = TRACKING_TABLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ensure_mesh_rt_staticlib();
    let url = database_url();
    let mut conn = native_pg_connect(&url).expect("the test database accepts connections");
    for sql in [
        "DROP TABLE IF EXISTS _mesh_migrations",
        "DROP TABLE IF EXISTS mesh_migrate_e2e_widgets",
    ] {
        native_pg_execute(&mut conn, sql, &[]).unwrap();
    }
    native_pg_close(conn);

    let project = tempfile::tempdir().unwrap();
    let project = project.path();
    write_migration(
        project,
        "29990101000001_create_widgets.mpl",
        "Pool.execute(pool, \"CREATE TABLE mesh_migrate_e2e_widgets (id BIGINT PRIMARY KEY)\", [])",
        "Pool.execute(pool, \"DROP TABLE mesh_migrate_e2e_widgets\", [])",
    );
    write_migration(
        project,
        "29990101000002_add_name.mpl",
        "Pool.execute(pool, \"ALTER TABLE mesh_migrate_e2e_widgets ADD COLUMN name TEXT\", [])",
        "Pool.execute(pool, \"ALTER TABLE mesh_migrate_e2e_widgets DROP COLUMN name\", [])",
    );
    let run = |args: &[&str]| {
        let output = migrate(project, args, Some(&url));
        (output.status.success(), command_output_text(&output))
    };

    let (ok, text) = run(&["status"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("[ ] 29990101000001_create_widgets") && text.contains("0 applied, 2 pending"),
        "{text}"
    );

    let (ok, text) = run(&["up"]);
    assert!(ok, "{text}");
    let first = text
        .find("Applied:  29990101000001_create_widgets")
        .expect(&text);
    let second = text.find("Applied:  29990101000002_add_name").expect(&text);
    assert!(
        first < second && text.contains("Applied 2 migration(s)"),
        "{text}"
    );

    let (ok, text) = run(&["status"]);
    assert!(ok && text.contains("[x] 29990101000002_add_name"), "{text}");
    assert!(text.contains("2 applied, 0 pending"), "{text}");
    let (ok, text) = run(&["up"]);
    assert!(ok && text.contains("No pending migrations"), "{text}");

    // A failing migration stops the run and is not recorded.
    write_migration(
        project,
        "29990101000003_fails.mpl",
        "Err(\"nope\")",
        "Ok(0)",
    );
    let (ok, text) = run(&["up"]);
    assert!(
        !ok && text.contains("Migration 29990101000003_fails failed: nope"),
        "{text}"
    );
    let (_, text) = run(&["status"]);
    assert!(text.contains("[ ] 29990101000003_fails"), "{text}");
    std::fs::remove_file(project.join("migrations/29990101000003_fails.mpl")).unwrap();

    // One whose program exits on its own, before reporting anything.
    write_migration(
        project,
        "29990101000003_exits.mpl",
        "Process.exit(3)\n  Ok(0)",
        "Ok(0)",
    );
    let (ok, text) = run(&["up"]);
    assert!(
        !ok && text.contains("Migration 29990101000003_exits exited with non-zero status"),
        "{text}"
    );
    std::fs::remove_file(project.join("migrations/29990101000003_exits.mpl")).unwrap();

    // One that does not compile names itself.
    write_migration(
        project,
        "29990101000004_broken.mpl",
        "undefined_thing()",
        "Ok(0)",
    );
    let (ok, text) = run(&["up"]);
    assert!(
        !ok && text.contains("Failed to compile migration 29990101000004_broken"),
        "{text}"
    );
    std::fs::remove_file(project.join("migrations/29990101000004_broken.mpl")).unwrap();

    let (ok, text) = run(&["down"]);
    assert!(
        ok && text.contains("Rolled back: 29990101000002_add_name"),
        "{text}"
    );
    // The file of the last applied version must exist to roll it back.
    let moved = project.join("create_widgets.mpl");
    std::fs::rename(
        project.join("migrations/29990101000001_create_widgets.mpl"),
        &moved,
    )
    .unwrap();
    let (ok, text) = run(&["down"]);
    assert!(
        !ok && text.contains("Migration file for version 29990101000001 not found"),
        "{text}"
    );
    std::fs::rename(
        &moved,
        project.join("migrations/29990101000001_create_widgets.mpl"),
    )
    .unwrap();
    let (ok, text) = run(&["down"]);
    assert!(
        ok && text.contains("Rolled back: 29990101000001_create_widgets"),
        "{text}"
    );
    let (ok, text) = run(&["down"]);
    assert!(ok && text.contains("No migrations to roll back"), "{text}");

    // A database that cannot be reached is an error, for every action.
    for action in ["up", "down", "status"] {
        let output = migrate(
            project,
            &[action],
            Some("postgres://nobody:nothing@127.0.0.1:1/none"),
        );
        assert!(!output.status.success(), "{action}");
        assert!(
            command_output_text(&output).contains("Failed to connect to database"),
            "{action}: {}",
            command_output_text(&output)
        );
    }
}

/// The tests that use the one tracking table, one at a time.
static TRACKING_TABLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `statements` against the test database.
fn execute(url: &str, statements: &[&str]) {
    let mut conn = native_pg_connect(url).expect("the test database accepts connections");
    for sql in statements {
        native_pg_execute(&mut conn, sql, &[]).unwrap_or_else(|error| panic!("{sql}: {error}"));
    }
    native_pg_close(conn);
}

/// A tracking table that is not `meshc migrate`'s, or one a migration
/// drops, stops the run naming what could not be read or recorded.
#[test]
#[ignore = "requires MESH_TEST_DATABASE_URL or the documented local mesh_test PostgreSQL"]
fn a_broken_tracking_table_stops_the_run() {
    let _tracking = TRACKING_TABLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ensure_mesh_rt_staticlib();
    let url = database_url();
    let project = tempfile::tempdir().unwrap();
    let project = project.path();
    write_migration(
        project,
        "29990201000001_drops_tracking.mpl",
        "Pool.execute(pool, \"DROP TABLE _mesh_migrations\", [])",
        "Pool.execute(pool, \"DROP TABLE _mesh_migrations\", [])",
    );
    let run = |action: &str| {
        let output = migrate(project, &[action], Some(&url));
        (output.status.success(), command_output_text(&output))
    };

    // Another table by that name, with no versions to read.
    execute(
        &url,
        &[
            "DROP TABLE IF EXISTS _mesh_migrations",
            "CREATE TABLE _mesh_migrations (id BIGINT)",
        ],
    );
    let (ok, text) = run("status");
    assert!(!ok && text.contains("version"), "{text}");

    // The migration runs, and then there is nowhere to record it.
    execute(&url, &["DROP TABLE _mesh_migrations"]);
    let (ok, text) = run("up");
    assert!(
        !ok && text.contains("Failed to record migration 29990201000001_drops_tracking"),
        "{text}"
    );

    // Rolled back, with nothing left to remove its row from.
    execute(
        &url,
        &[
            "CREATE TABLE _mesh_migrations (version BIGINT PRIMARY KEY, name TEXT NOT NULL, \
             applied_at TIMESTAMPTZ NOT NULL DEFAULT now())",
            "INSERT INTO _mesh_migrations (version, name) VALUES (29990201000001, 'drops_tracking')",
        ],
    );
    let (ok, text) = run("down");
    assert!(
        !ok && text.contains("Failed to remove tracking row for version 29990201000001"),
        "{text}"
    );
    execute(&url, &["DROP TABLE IF EXISTS _mesh_migrations"]);
}
