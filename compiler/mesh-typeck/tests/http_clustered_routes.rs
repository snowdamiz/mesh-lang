use mesh_typeck::diagnostics::DiagnosticOptions;
use mesh_typeck::error::TypeError;
use mesh_typeck::ty::{Scheme, Ty, TyCon};
use mesh_typeck::{
    ClusteredRouteReplicationCountSource, ImportContext, ModuleExports, TypeckResult,
};
use rustc_hash::{FxHashMap, FxHashSet};

fn handler_ty() -> Ty {
    Ty::fun(
        vec![Ty::Con(TyCon::new("Request"))],
        Ty::Con(TyCon::new("Response")),
    )
}

fn handler_scheme() -> Scheme {
    Scheme::mono(handler_ty())
}

fn check_source(src: &str, import_ctx: ImportContext) -> TypeckResult {
    let parse = mesh_parser::parse(src);
    mesh_typeck::check_with_imports(&parse, &import_ctx)
}

fn check_source_in_module(src: &str, module_name: &str) -> TypeckResult {
    let mut import_ctx = ImportContext::empty();
    import_ctx.current_module = Some(module_name.to_string());
    check_source(src, import_ctx)
}

fn module_exports(
    module_name: &str,
    exported_handlers: &[&str],
    private_handlers: &[&str],
) -> ModuleExports {
    let mut functions = FxHashMap::default();
    for handler in exported_handlers {
        functions.insert((*handler).to_string(), handler_scheme());
    }

    let mut private_names = FxHashSet::default();
    for handler in private_handlers {
        private_names.insert((*handler).to_string());
    }

    ModuleExports {
        module_name: module_name.to_string(),
        functions,
        struct_defs: FxHashMap::default(),
        sum_type_defs: FxHashMap::default(),
        service_defs: FxHashMap::default(),
        actor_defs: FxHashMap::default(),
        private_names,
        type_aliases: FxHashMap::default(),
        ..ModuleExports::default()
    }
}

fn metadata_by_runtime_name<'a>(
    result: &'a TypeckResult,
    runtime_name: &str,
) -> &'a mesh_typeck::ClusteredRouteWrapperMetadata {
    result
        .clustered_route_wrappers
        .values()
        .find(|metadata| metadata.runtime_name == runtime_name)
        .unwrap_or_else(|| panic!("missing clustered route metadata for {runtime_name:?}"))
}

fn assert_no_generic_wrapper_noise(result: &TypeckResult) {
    assert!(
        !result.errors.iter().any(|error| matches!(
            error,
            TypeError::UnboundVariable { .. } | TypeError::NoSuchField { .. }
        )),
        "expected wrapper-specific diagnostics, got: {:?}",
        result.errors
    );
}

#[test]
fn clustered_route_wrapper_accepts_direct_and_pipe_forms_and_tracks_counts() {
    let src = r#"
import Api.Todos

pub fn handle_local(req :: Request) -> Response do
  HTTP.response(200, "ok")
end

fn build() do
  let router = HTTP.router()
  let router = HTTP.on_get(router, "/local", HTTP.clustered(handle_local))
  router |> HTTP.on_get("/todos", HTTP.clustered(3, Todos.handle_list_todos))
end
"#;

    let mut import_ctx = ImportContext::empty();
    import_ctx.current_module = Some("App.Router".to_string());
    import_ctx.module_exports.insert(
        "Todos".to_string(),
        module_exports("Api.Todos", &["handle_list_todos"], &[]),
    );

    let result = check_source(src, import_ctx);
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
    assert_eq!(result.clustered_route_wrappers.len(), 2);

    let local = metadata_by_runtime_name(&result, "App.Router.handle_local");
    assert_eq!(local.handler_name, "handle_local");
    assert_eq!(local.defining_module.as_deref(), Some("App.Router"));
    assert_eq!(local.replication_count.value, 2);
    assert_eq!(
        local.replication_count.source,
        ClusteredRouteReplicationCountSource::Default
    );

    let imported = metadata_by_runtime_name(&result, "Api.Todos.handle_list_todos");
    assert_eq!(imported.handler_name, "handle_list_todos");
    assert_eq!(imported.defining_module.as_deref(), Some("Api.Todos"));
    assert_eq!(imported.replication_count.value, 3);
    assert_eq!(
        imported.replication_count.source,
        ClusteredRouteReplicationCountSource::Explicit
    );
}

#[test]
fn clustered_route_wrapper_preserves_imported_bare_handler_origin() {
    let src = r#"
from Api.Todos import handle_list_todos

fn build() do
  HTTP.router() |> HTTP.on_get("/todos", HTTP.clustered(handle_list_todos))
end
"#;

    let mut import_ctx = ImportContext::empty();
    import_ctx.current_module = Some("App.Router".to_string());
    import_ctx.module_exports.insert(
        "Todos".to_string(),
        module_exports("Api.Todos", &["handle_list_todos"], &[]),
    );

    let result = check_source(src, import_ctx);
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
    assert_eq!(result.clustered_route_wrappers.len(), 1);

    let metadata = metadata_by_runtime_name(&result, "Api.Todos.handle_list_todos");
    assert_eq!(metadata.defining_module.as_deref(), Some("Api.Todos"));
    assert_eq!(metadata.replication_count.value, 2);
    assert_eq!(
        metadata.replication_count.source,
        ClusteredRouteReplicationCountSource::Default
    );
}

#[test]
fn clustered_route_wrapper_rejects_non_route_position() {
    let src = r#"
pub fn handle(req :: Request) -> Response do
  HTTP.response(200, "ok")
end

fn build() do
  let wrapped = HTTP.clustered(handle)
  wrapped
end
"#;

    let result = check_source_in_module(src, "App.Router");
    assert_no_generic_wrapper_noise(&result);
    assert!(
        result.errors.iter().any(|error| {
            matches!(
                error,
                TypeError::HttpClusteredOutsideRouteHandlerPosition { .. }
            )
        }),
        "expected non-route-position error, got: {:?}",
        result.errors
    );

    let rendered = result.render_errors(src, "test.mpl", &DiagnosticOptions::colorless());
    assert!(
        rendered
            .iter()
            .any(|diag| diag.contains("E0049") && diag.contains("route handler position")),
        "expected focused non-route-position diagnostic, got: {:?}",
        rendered
    );
}

#[test]
fn clustered_route_wrapper_rejects_closure_handler() {
    let src = r#"
fn build() do
  HTTP.router() |> HTTP.on_get("/x", HTTP.clustered(fn (req) -> req end))
end
"#;

    let result = check_source_in_module(src, "App.Router");
    assert_no_generic_wrapper_noise(&result);
    assert!(
        result.errors.iter().any(|error| {
            matches!(
                error,
                TypeError::HttpClusteredInvalidArguments { reason, .. }
                    if reason.contains("bare handler reference")
            )
        }),
        "expected invalid handler reference error, got: {:?}",
        result.errors
    );
}

#[test]
fn clustered_route_wrapper_rejects_private_handler() {
    let src = r#"
fn hidden(req :: Request) -> Response do
  HTTP.response(200, "ok")
end

fn build() do
  HTTP.router() |> HTTP.on_get("/x", HTTP.clustered(hidden))
end
"#;

    let result = check_source_in_module(src, "App.Router");
    assert_no_generic_wrapper_noise(&result);
    assert!(
        result.errors.iter().any(|error| {
            matches!(
                error,
                TypeError::HttpClusteredPrivateHandler { handler_name, .. }
                    if handler_name == "hidden"
            )
        }),
        "expected private-handler error, got: {:?}",
        result.errors
    );
}

#[test]
fn clustered_route_wrapper_rejects_conflicting_replication_counts() {
    let src = r#"
pub fn handle(req :: Request) -> Response do
  HTTP.response(200, "ok")
end

fn build() do
  let router = HTTP.router()
  let router = HTTP.on_get(router, "/one", HTTP.clustered(handle))
  HTTP.on_get(router, "/two", HTTP.clustered(3, handle))
end
"#;

    let result = check_source_in_module(src, "App.Router");
    assert_no_generic_wrapper_noise(&result);
    assert!(
        result.errors.iter().any(|error| {
            matches!(
                error,
                TypeError::HttpClusteredConflictingReplicationCount {
                    first_count,
                    current_count,
                    ..
                } if *first_count == 2 && *current_count == 3
            )
        }),
        "expected conflicting-count error, got: {:?}",
        result.errors
    );
}

#[test]
fn clustered_route_wrapper_rejects_imported_origin_drift() {
    let src = r#"
from Api.Todos import handle_list_todos

fn build() do
  HTTP.router() |> HTTP.on_get("/todos", HTTP.clustered(handle_list_todos))
end
"#;

    let mut import_ctx = ImportContext::empty();
    import_ctx.current_module = Some("App.Router".to_string());
    import_ctx.module_exports.insert(
        "Todos".to_string(),
        module_exports("", &["handle_list_todos"], &[]),
    );

    let result = check_source(src, import_ctx);
    assert_no_generic_wrapper_noise(&result);
    assert!(
        result.errors.iter().any(|error| {
            matches!(
                error,
                TypeError::HttpClusteredImportedOriginMissing { handler_name, .. }
                    if handler_name == "handle_list_todos"
            )
        }),
        "expected imported-origin diagnostic, got: {:?}",
        result.errors
    );
}

/// Each way a handler reference can be wrong is its own error: of the
/// wrong type (here, imported or qualified), defined at several arities, a
/// local binding, no name at all, a module that is not imported, a private
/// or unexported name, or a qualified name on a computed value.
#[test]
fn clustered_route_wrapper_rejects_each_bad_handler_reference() {
    let mut todos = module_exports("Api.Todos", &["handle", "hidden"], &["secret"]);
    todos.functions.insert(
        "count".to_string(),
        Scheme::mono(Ty::fun(vec![Ty::int()], Ty::int())),
    );
    let check = |defs: &str, handler: &str| {
        let mut import_ctx = ImportContext::empty();
        import_ctx.current_module = Some("App.Router".to_string());
        import_ctx
            .module_exports
            .insert("Todos".to_string(), todos.clone());
        let src = format!(
            "import Api.Todos\nfrom Api.Todos import count\n\n{defs}\n\nfn build() do\n  HTTP.router() |> HTTP.on_get(\"/x\", HTTP.clustered({handler}))\nend\n"
        );
        check_source(&src, import_ctx)
    };
    let invalid = |result: &TypeckResult, expected: &str| {
        assert!(
            result.errors.iter().any(|error| matches!(
                error,
                TypeError::HttpClusteredInvalidArguments { reason, .. } if reason.contains(expected)
            )),
            "{expected}: {:?}",
            result.errors
        );
    };
    let local = "pub fn add(x :: Int) -> Int do\n  x\nend";
    invalid(
        &check(local, "add"),
        "`add` must have type `(Request) -> Response`",
    );
    invalid(&check("", "count"), "`count` must have type");
    invalid(&check("", "Todos.count"), "`Todos.count` must have type");
    let overloaded = "pub fn h(req :: Request) -> Response do\n  HTTP.response(200, \"a\")\nend\npub fn h(a :: Int, b :: Int) -> Int do\n  a\nend";
    invalid(
        &check(overloaded, "h"),
        "must resolve to a single public top-level",
    );
    invalid(
        &check("", "nothing"),
        "`nothing` must resolve to a public top-level",
    );
    invalid(
        &check("", "Nope.handle"),
        "must reference a function from an imported user module",
    );
    invalid(
        &check("", "Todos.missing"),
        "is not an exported public handler",
    );
    invalid(
        &check("", "build().handle"),
        "expected a module-qualified handler reference",
    );
    // A replication count is a positive integer literal, before the handler.
    for count in ["0", "\"3\"", "build()"] {
        invalid(
            &check("", &format!("{count}, Todos.handle")),
            "the replication count must be a positive integer literal",
        );
    }
    for args in ["", "1, 2, Todos.handle"] {
        invalid(
            &check("", args),
            "expected `HTTP.clustered(handler)` or `HTTP.clustered(<int>, handler)`",
        );
    }
    let private = check("", "Todos.secret");
    assert!(
        private.errors.iter().any(|error| matches!(
            error,
            TypeError::HttpClusteredPrivateHandler { handler_name, .. } if handler_name == "secret"
        )),
        "{:?}",
        private.errors
    );
    let fine = check("", "Todos.handle");
    assert!(fine.errors.is_empty(), "{:?}", fine.errors);
}

/// `Continuity.promote` is refused wherever it is named, a clustered
/// route's handler too, even through a module of that name.
#[test]
fn clustered_route_wrapper_refuses_manual_promotion_as_a_handler() {
    let mut import_ctx = ImportContext::empty();
    import_ctx.current_module = Some("App.Router".to_string());
    import_ctx.module_exports.insert(
        "Continuity".to_string(),
        module_exports("Ops.Continuity", &["promote"], &[]),
    );
    let result = check_source(
        "import Ops.Continuity\n\nfn build() do\n  HTTP.router() |> HTTP.on_get(\"/p\", HTTP.clustered(Continuity.promote))\nend\n",
        import_ctx,
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| matches!(error, TypeError::ManualContinuityPromotionDisabled { .. })),
        "{:?}",
        result.errors
    );
}

/// In a program checked alone, in no module, a handler runs under its own
/// name; a local or top-level binding of it, a function of two parameters
/// and a count that is no integer are each an error.
#[test]
fn clustered_route_wrapper_in_no_module() {
    let src = r#"
pub fn handle(req :: Request) -> Response do
  HTTP.response(200, "ok")
end

pub fn two(req :: Request, n :: Int) -> Response do
  HTTP.response(200, "ok")
end

let top = handle

fn local_binding() do
  let local = handle
  HTTP.router() |> HTTP.on_get("/a", HTTP.clustered(local))
end

fn two_params() do
  HTTP.router() |> HTTP.on_get("/b", HTTP.clustered(two))
end

fn float_count() do
  HTTP.router() |> HTTP.on_get("/c", HTTP.clustered(1.5, handle))
end

fn top_level() do
  HTTP.router() |> HTTP.on_get("/d", HTTP.clustered(top))
end

fn fine() do
  HTTP.router() |> HTTP.on_get("/e", HTTP.clustered(handle))
end
"#;
    let result = check_source(src, ImportContext::empty());
    let reasons: Vec<&str> = result
        .errors
        .iter()
        .filter_map(|error| match error {
            TypeError::HttpClusteredInvalidArguments { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        [
            "`local` must be a public top-level function reference, not a local binding or unsupported value",
            "`two` must have type `(Request) -> Response`",
            "the replication count must be a positive integer literal, e.g. `HTTP.clustered(3, handler)`",
            "`top` must be a public top-level function reference, not a local binding or unsupported value",
        ]
    );
    assert_eq!(result.errors.len(), 4, "{:?}", result.errors);
    let handler = metadata_by_runtime_name(&result, "handle");
    assert_eq!(handler.defining_module, None);
}
