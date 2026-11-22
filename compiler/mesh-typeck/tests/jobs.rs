use mesh_typeck::error::TypeError;
use mesh_typeck::{ImportContext, ModuleExports, TypeckResult};

fn check(source: &str) -> TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "{:?}", parse.errors());
    mesh_typeck::check(&parse)
}

#[test]
fn jobs_public_signatures() {
    let result = check(
        r#"
from Jobs import start

fn run(pool :: PoolHandle) -> Result<Int, String> do
  pool |> start("{}", handle)
end

fn handle(job :: String) do job end
fn migrate(pool :: PoolHandle) -> Result<Unit, String> do Jobs.migrate(pool) end
fn enqueue(pool :: PoolHandle) -> Result<String, String> do Jobs.enqueue(pool, "app", "{}") end
fn control(pool :: PoolHandle) -> Result<String, String> do Jobs.control(pool, "app", "{}") end
fn stop(runner :: Int) -> Result<String, String> do Jobs.stop(runner, 100) end
fn join(runner :: Int) -> Result<String, String> do Jobs.join(runner) end
fn status(runner :: Int) -> Result<String, String> do Jobs.status(runner) end
fn events(runner :: Int) -> Result<String, String> do Jobs.read_events(runner, 32) end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn jobs_start_accepts_named_handlers_from_another_module() {
    let handlers = mesh_parser::parse("pub fn handle(job :: String) -> String do job end");
    let exports = mesh_typeck::collect_exports(&handlers, &mesh_typeck::check(&handlers));
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Workers".to_string(),
        ModuleExports::new("Workers".to_string(), &exports),
    );
    let source = mesh_parser::parse(
        r#"
import Workers
from Workers import handle

fn run(pool :: PoolHandle) do
  let _ = Jobs.start(pool, "{}", Workers.handle)
  handle |3> Jobs.start(pool, "{}")
end
"#,
    );
    assert!(source.ok(), "{:?}", source.errors());
    let result = mesh_typeck::check_with_imports(&source, &imports);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn jobs_start_rejects_closures_generics_and_indirect_calls() {
    for expression in [
        r#"Jobs.start(pool, "{}", fn(job) do job end)"#,
        r#"Jobs.start(pool, "{}", fn(_job) do config end)"#,
        r#"Jobs.start(pool, "{}", handler_alias)"#,
        r#"Jobs.start(pool, "{}", generic)"#,
        r#"Jobs.start(pool, "{}", wrong_type)"#,
        r#"Jobs.start(pool, "{}", handle)"#,
        "Jobs.start",
        "start",
        "jobs_start",
    ] {
        let result = check(&format!(
            r#"
from Jobs import start

fn generic(job) do job end
fn wrong_type(job :: Int) -> Int do job end
fn handle(job :: String) -> String do job end
fn run(pool :: PoolHandle, config :: String) do
  let handler_alias = handle
  let handle = handler_alias
  {expression}
end
"#
        ));
        assert!(
            result
                .errors
                .iter()
                .any(|error| matches!(error, TypeError::JobsStartInvalid { .. })),
            "{expression}: {:?}",
            result.errors
        );
    }
}

#[test]
fn jobs_enqueue_in_keeps_transaction_connection_borrowed() {
    let result = check(
        r#"
from Jobs import enqueue_in

fn transaction_body(conn :: borrow PgConn) -> Result<String, String> do
  let _ = Pg.execute(conn, "SELECT 1", [])
  let _ = Jobs.enqueue_in(conn, "app", "{}")
  let _ = conn |> enqueue_in("app", "{}")
  jobs_enqueue_in(conn, "app", "{}")
end

fn run(pool :: PoolHandle) do Repo.transaction(pool, transaction_body) end
"#,
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn jobs_enqueue_in_rejects_escaped_or_consumed_connections() {
    use mesh_diagnostics::Code;

    for (body, message, code) in [
        (
            r#"fn() do Jobs.enqueue_in(conn, "app", "{}") end"#,
            "captured",
            Code::ResourceCrossesBoundary,
        ),
        (
            r#"let enqueue_in = Jobs.enqueue_in
  enqueue_in(conn, "app", "{}")"#,
            "indirect call",
            Code::ResourceCrossesBoundary,
        ),
        (
            r#"Pg.close(conn)
  Jobs.enqueue_in(conn, "app", "{}")"#,
            "used after it moved",
            Code::ResourceOwnershipViolation,
        ),
    ] {
        let ownership = if code == Code::ResourceOwnershipViolation {
            ""
        } else {
            "borrow "
        };
        let result = check(&format!(
            "from Jobs import enqueue_in\n\nfn run(conn :: {ownership}PgConn) do\n  {body}\nend"
        ));
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.code() == code && error.to_string().contains(message)),
            "{body}: {:?}",
            result.errors
        );
    }
}

#[test]
fn durable_drain_requires_a_named_compiled_handler() {
    for expression in [
        r#"Jobs.drain(pool, "{}", handle, "{}")"#,
        r#"pool |> drain("{}", handle, "{}")"#,
        r#"handle |3> Jobs.drain(pool, "{}", "{}")"#,
    ] {
        let checked = check(&format!(
            "from Jobs import drain\nfn handle(job :: String) -> String do job end\nfn run(pool :: PoolHandle) do {expression} end"
        ));
        assert!(
            checked.errors.is_empty(),
            "{expression}: {:?}",
            checked.errors
        );
    }
    for expression in [
        r#"Jobs.drain(pool, "{}", fn(job) do job end, "{}")"#,
        "drain",
    ] {
        let checked = check(&format!(
            "from Jobs import drain\nfn run(pool :: PoolHandle) do {expression} end"
        ));
        assert!(
            checked
                .errors
                .iter()
                .any(|error| matches!(error, TypeError::JobsStartInvalid { .. })),
            "{expression}: {:?}",
            checked.errors
        );
    }
}

#[test]
fn job_test_functions_require_the_test_capability() {
    for name in [
        "jobs_inspect",
        "jobs_inspect_in",
        "assert_enqueued",
        "refute_enqueued",
        "assert_enqueued_in",
        "refute_enqueued_in",
        "build_job",
        "perform_job",
    ] {
        let source = format!("from Test import {name}\nfn use_it() do Test.{name} end");
        let source = mesh_parser::parse(&source);
        assert!(source.ok(), "{:?}", source.errors());
        assert!(!mesh_typeck::check(&source).errors.is_empty(), "{name}");
        let enabled = mesh_typeck::check_with_imports(
            &source,
            &ImportContext {
                test_builtins: true,
                ..ImportContext::default()
            },
        );
        assert!(enabled.errors.is_empty(), "{name}: {:?}", enabled.errors);
    }
}

#[test]
fn job_test_assertions_borrow_the_transaction_connection() {
    use mesh_diagnostics::Code;

    let imports = ImportContext {
        test_builtins: true,
        ..ImportContext::default()
    };
    let source = mesh_parser::parse(
        r#"
from Test import assert_enqueued_in
fn check(conn :: borrow PgConn) do
  let _ = Test.jobs_inspect_in(conn, "namespace", "{}")
  let _ = conn |> assert_enqueued_in("namespace", "{}")
  let _ = Test.refute_enqueued_in(conn, "namespace", "{}")
  Pg.execute(conn, "SELECT 1", [])
end
"#,
    );
    let checked = mesh_typeck::check_with_imports(&source, &imports);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let source = mesh_parser::parse(
        r#"
fn escape(conn :: borrow PgConn) do
  fn() do Test.assert_enqueued_in(conn, "namespace", "{}") end
end
"#,
    );
    let checked = mesh_typeck::check_with_imports(&source, &imports);
    assert!(
        checked.errors.iter().any(|error| {
            error.code() == Code::ResourceCrossesBoundary && error.to_string().contains("captured")
        }),
        "{:?}",
        checked.errors
    );
}

#[test]
fn jobs_start_effects_checks_the_third_argument_as_a_direct_handler() {
    for (source, should_pass) in [
        (
            r#"Jobs.start_effects(pool, "{}", handler, "url", registry)"#,
            true,
        ),
        (
            r#"handler |3> start_effects(pool, "{}", "url", registry)"#,
            true,
        ),
        (
            r#"Jobs.start_effects(pool, "{}", fn(input) do input end, "url", registry)"#,
            false,
        ),
        ("Jobs.start_effects", false),
        ("start_effects", false),
    ] {
        let checked = check(&format!(
            "from Jobs import start_effects\nfn handler(input :: String) -> String do input end\n\
             fn run(pool :: PoolHandle, registry :: Json) do {source} end"
        ));
        if should_pass {
            assert!(checked.errors.is_empty(), "{source}: {:?}", checked.errors);
        } else {
            assert!(
                checked
                    .errors
                    .iter()
                    .any(|error| matches!(error, TypeError::JobsStartInvalid { .. })),
                "{source}: {:?}",
                checked.errors
            );
        }
    }
}

#[test]
fn compiled_jobs_effect_fixture_typechecks() {
    let checked = check(include_str!(
        "../../../scripts/fixtures/jobs-effect-compiled/main.mpl"
    ));
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
}
