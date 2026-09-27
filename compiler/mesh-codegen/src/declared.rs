use std::collections::BTreeMap;

use mesh_typeck::TypeckResult;

use crate::mir::{MirExpr, MirFunction, MirModule, MirType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredHandlerKind {
    Work,
    Route,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredHandlerPlanEntry {
    pub kind: DeclaredHandlerKind,
    pub runtime_registration_name: String,
    pub executable_symbol: String,
    pub replication_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredRuntimeRegistration {
    pub runtime_registration_name: String,
    pub executable_symbol: String,
    pub replication_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupWorkRegistration {
    pub runtime_registration_name: String,
}

/// `default_replicas` is the count for `HTTP.clustered(handler)` without one:
/// the manifest's `[cluster].default_replicas`.
pub fn prepare_clustered_route_handler_plan<'a>(
    typecks: impl IntoIterator<Item = &'a TypeckResult>,
    default_replicas: u32,
) -> Result<Vec<DeclaredHandlerPlanEntry>, String> {
    let mut planned_routes = BTreeMap::<String, u64>::new();

    for typeck in typecks {
        for metadata in typeck.clustered_route_wrappers.values() {
            // The runtime name is the handler's qualified name, and a count
            // is positive: the checker takes an explicit one only as a
            // positive literal and the manifest's default only from 1.
            let runtime_registration_name = metadata.runtime_name.as_str();
            let replication_count = u64::from(match metadata.replication_count.source {
                mesh_typeck::ClusteredRouteReplicationCountSource::Default => default_replicas,
                mesh_typeck::ClusteredRouteReplicationCountSource::Explicit => {
                    metadata.replication_count.value
                }
            });

            if let Some(existing_count) = planned_routes.get(runtime_registration_name) {
                if *existing_count != replication_count {
                    return Err(format!(
                        "clustered route handler `{runtime_registration_name}` lowered with conflicting replication counts {existing_count} and {replication_count}"
                    ));
                }
                continue;
            }

            planned_routes.insert(runtime_registration_name.to_string(), replication_count);
        }
    }

    Ok(planned_routes
        .into_iter()
        .map(
            |(runtime_registration_name, replication_count)| DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Route,
                executable_symbol: declared_route_wrapper_name(&runtime_registration_name),
                runtime_registration_name,
                replication_count,
            },
        )
        .collect())
}

pub fn prepare_startup_work_registrations(
    plan: &[DeclaredHandlerPlanEntry],
) -> Vec<StartupWorkRegistration> {
    plan.iter()
        .filter(|entry| entry.kind == DeclaredHandlerKind::Work)
        .map(|entry| StartupWorkRegistration {
            runtime_registration_name: entry.runtime_registration_name.clone(),
        })
        .collect()
}

pub fn prepare_declared_runtime_handlers(
    mir: &mut MirModule,
    plan: &[DeclaredHandlerPlanEntry],
) -> Result<Vec<DeclaredRuntimeRegistration>, String> {
    let mut registrations = Vec::with_capacity(plan.len());

    for entry in plan {
        let executable_symbol = match entry.kind {
            DeclaredHandlerKind::Work => generate_declared_work_wrapper(
                mir,
                &entry.runtime_registration_name,
                &entry.executable_symbol,
            )?,
            DeclaredHandlerKind::Route => validate_declared_route_wrapper(
                mir,
                &entry.runtime_registration_name,
                &entry.executable_symbol,
            )?,
        };

        registrations.push(DeclaredRuntimeRegistration {
            runtime_registration_name: entry.runtime_registration_name.clone(),
            executable_symbol,
            replication_count: entry.replication_count,
        });
    }

    Ok(registrations)
}

fn generate_declared_work_wrapper(
    mir: &mut MirModule,
    runtime_registration_name: &str,
    executable_symbol: &str,
) -> Result<String, String> {
    let original = mir
        .functions
        .iter()
        .find(|func| func.name == executable_symbol)
        .ok_or_else(|| {
            format!(
                "declared work target `{runtime_registration_name}` has no lowered function `{executable_symbol}`"
            )
        })?;
    if !original.params.is_empty() {
        return Err(format!(
            "declared work target `{runtime_registration_name}` must use `pub fn name() -> ...`; continuity metadata is runtime-owned"
        ));
    }
    let return_type = original.return_type.clone();

    // The actor body runs the work and drops its result; the runtime owns the
    // continuity metadata it receives.
    let wrapper_name = declared_work_wrapper_name(runtime_registration_name);
    let call = MirExpr::Call {
        func: Box::new(MirExpr::Var(
            executable_symbol.to_string(),
            MirType::FnPtr(Vec::new(), Box::new(return_type.clone())),
        )),
        args: Vec::new(),
        ty: return_type,
    };
    mir.functions.push(MirFunction {
        name: format!("__actor_{wrapper_name}_body"),
        params: vec![
            ("request_key".to_string(), MirType::String),
            ("attempt_id".to_string(), MirType::String),
        ],
        return_type: MirType::Unit,
        body: MirExpr::Block(vec![call, MirExpr::Unit], MirType::Unit),
        is_closure_fn: false,
        captures: Vec::new(),
        has_tail_calls: false,
    });
    mir.functions.push(MirFunction {
        name: wrapper_name.clone(),
        params: vec![("__args_ptr".to_string(), MirType::Ptr)],
        return_type: MirType::Unit,
        body: MirExpr::Unit,
        is_closure_fn: false,
        captures: Vec::new(),
        has_tail_calls: false,
    });

    Ok(wrapper_name)
}

/// A route's shim is generated where lowering meets its `HTTP.clustered`
/// wrapper, always as `fn(Request) -> Response`.
fn validate_declared_route_wrapper(
    mir: &MirModule,
    runtime_registration_name: &str,
    executable_symbol: &str,
) -> Result<String, String> {
    if mir
        .functions
        .iter()
        .any(|func| func.name == executable_symbol)
    {
        Ok(executable_symbol.to_string())
    } else {
        Err(format!(
            "declared route target `{runtime_registration_name}` has no lowered function `{executable_symbol}`"
        ))
    }
}

pub fn declared_route_wrapper_name(runtime_registration_name: &str) -> String {
    format!(
        "__declared_route_{}",
        sanitize_runtime_name(runtime_registration_name)
    )
}

fn declared_work_wrapper_name(runtime_registration_name: &str) -> String {
    format!(
        "__declared_work_{}",
        sanitize_runtime_name(runtime_registration_name)
    )
}

fn sanitize_runtime_name(runtime_registration_name: &str) -> String {
    runtime_registration_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rustc_hash::{FxHashMap, FxHashSet};

    use super::*;
    use mesh_typeck::ty::{Scheme, Ty, TyCon};
    use mesh_typeck::{ImportContext, ModuleExports};

    fn empty_module() -> MirModule {
        MirModule {
            functions: Vec::new(),
            structs: Vec::new(),
            sum_types: Vec::new(),
            entry_function: None,
            service_dispatch: std::collections::HashMap::new(),
            actors: Vec::new(),
            native_functions: Vec::new(),
        }
    }

    fn route_handler_ty() -> Ty {
        Ty::fun(
            vec![Ty::Con(TyCon::new("Request"))],
            Ty::Con(TyCon::new("Response")),
        )
    }

    fn route_handler_scheme() -> Scheme {
        Scheme::mono(route_handler_ty())
    }

    fn route_module_exports(module_name: &str, exported_handlers: &[&str]) -> ModuleExports {
        let mut functions = FxHashMap::default();
        for handler in exported_handlers {
            functions.insert((*handler).to_string(), route_handler_scheme());
        }

        ModuleExports {
            module_name: module_name.to_string(),
            functions,
            struct_defs: FxHashMap::default(),
            sum_type_defs: FxHashMap::default(),
            service_defs: FxHashMap::default(),
            actor_defs: FxHashMap::default(),
            private_names: FxHashSet::default(),
            type_aliases: FxHashMap::default(),
            ..ModuleExports::default()
        }
    }

    fn typecheck_routes_in_module(source: &str, module_name: &str) -> TypeckResult {
        let parse = mesh_parser::parse(source);
        let mut import_ctx = ImportContext::empty();
        import_ctx.current_module = Some(module_name.to_string());
        mesh_typeck::check_with_imports(&parse, &import_ctx)
    }

    fn typecheck_routes_with_imports(
        source: &str,
        current_module: &str,
        imported_module: &str,
        imported_handler: &str,
    ) -> TypeckResult {
        let parse = mesh_parser::parse(source);
        let mut import_ctx = ImportContext::empty();
        import_ctx.current_module = Some(current_module.to_string());
        import_ctx.module_exports.insert(
            imported_module
                .rsplit('.')
                .next()
                .unwrap_or(imported_module)
                .to_string(),
            route_module_exports(imported_module, &[imported_handler]),
        );
        mesh_typeck::check_with_imports(&parse, &import_ctx)
    }

    #[test]
    fn startup_work_registrations_filter_out_route_handlers() {
        let registrations = prepare_startup_work_registrations(&[
            DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Work,
                runtime_registration_name: "Work.handle_submit".to_string(),
                executable_symbol: "handle_submit".to_string(),
                replication_count: 2,
            },
            DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Route,
                runtime_registration_name: "Api.Todos.handle_list_todos".to_string(),
                executable_symbol: "__declared_route_api_todos_handle_list_todos".to_string(),
                replication_count: 2,
            },
        ]);

        assert_eq!(
            registrations,
            vec![StartupWorkRegistration {
                runtime_registration_name: "Work.handle_submit".to_string(),
            }]
        );
    }

    #[test]
    fn clustered_route_handler_plan_dedupes_identical_wrappers() {
        let typeck = typecheck_routes_in_module(
            r#"
pub fn handle(req :: Request) -> Response do
  HTTP.response(200, "ok")
end

fn build() do
  let router = HTTP.router()
  let router = HTTP.on_get(router, "/one", HTTP.clustered(handle))
  router |> HTTP.on_get("/two", HTTP.clustered(handle))
end
"#,
            "App.Router",
        );
        assert!(
            typeck.errors.is_empty(),
            "expected route wrapper source to type-check cleanly, got {:?}",
            typeck.errors
        );

        let plan = prepare_clustered_route_handler_plan([&typeck], 2)
            .expect("identical clustered route wrappers should dedupe cleanly");

        assert_eq!(
            plan,
            vec![DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Route,
                runtime_registration_name: "App.Router.handle".to_string(),
                executable_symbol: "__declared_route_app_router_handle".to_string(),
                replication_count: 2,
            }]
        );
    }

    #[test]
    fn clustered_route_handler_plan_rejects_conflicting_replication_counts_across_modules() {
        let defaulted = typecheck_routes_with_imports(
            r#"
from Api.Todos import handle_list_todos

fn build() do
  HTTP.router() |> HTTP.on_get("/todos", HTTP.clustered(handle_list_todos))
end
"#,
            "App.RouterOne",
            "Api.Todos",
            "handle_list_todos",
        );
        let explicit = typecheck_routes_with_imports(
            r#"
from Api.Todos import handle_list_todos

fn build() do
  HTTP.router() |> HTTP.on_get("/todos", HTTP.clustered(3, handle_list_todos))
end
"#,
            "App.RouterTwo",
            "Api.Todos",
            "handle_list_todos",
        );

        let error = prepare_clustered_route_handler_plan([&defaulted, &explicit], 2)
            .expect_err("conflicting imported route counts must fail closed");

        assert!(
            error.contains("Api.Todos.handle_list_todos")
                && error.contains("conflicting replication counts 2 and 3"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn declared_runtime_handlers_preserve_route_runtime_name_and_counts() {
        let mut mir = empty_module();
        mir.functions.push(MirFunction {
            name: "__declared_route_api_todos_handle_list_todos".to_string(),
            params: vec![("__request".to_string(), MirType::Ptr)],
            return_type: MirType::Ptr,
            body: MirExpr::Var("__request".to_string(), MirType::Ptr),
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls: false,
        });

        let registrations = prepare_declared_runtime_handlers(
            &mut mir,
            &[DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Route,
                runtime_registration_name: "Api.Todos.handle_list_todos".to_string(),
                executable_symbol: declared_route_wrapper_name("Api.Todos.handle_list_todos"),
                replication_count: 3,
            }],
        )
        .expect("route shims should register without startup wrappers");

        assert_eq!(
            registrations,
            vec![DeclaredRuntimeRegistration {
                runtime_registration_name: "Api.Todos.handle_list_todos".to_string(),
                executable_symbol: "__declared_route_api_todos_handle_list_todos".to_string(),
                replication_count: 3,
            }]
        );
    }

    #[test]
    fn declared_route_handlers_reject_missing_lowered_symbol_before_registration() {
        let mut mir = empty_module();

        let error = prepare_declared_runtime_handlers(
            &mut mir,
            &[DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Route,
                runtime_registration_name: "Api.Todos.handle_list_todos".to_string(),
                executable_symbol: declared_route_wrapper_name("Api.Todos.handle_list_todos"),
                replication_count: 2,
            }],
        )
        .expect_err("missing lowered route shim must fail before registration");

        assert!(
            error.contains(
                "declared route target `Api.Todos.handle_list_todos` has no lowered function `__declared_route_api_todos_handle_list_todos`"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn declared_runtime_handlers_preserve_default_and_explicit_replication_counts() {
        let mut mir = empty_module();
        mir.functions.push(MirFunction {
            name: "handle_submit".to_string(),
            params: vec![],
            return_type: MirType::Int,
            body: MirExpr::IntLit(0, MirType::Int),
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls: false,
        });
        mir.functions.push(MirFunction {
            name: "handle_retry".to_string(),
            params: vec![],
            return_type: MirType::Int,
            body: MirExpr::IntLit(1, MirType::Int),
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls: false,
        });

        let registrations = prepare_declared_runtime_handlers(
            &mut mir,
            &[
                DeclaredHandlerPlanEntry {
                    kind: DeclaredHandlerKind::Work,
                    runtime_registration_name: "Work.handle_submit".to_string(),
                    executable_symbol: "handle_submit".to_string(),
                    replication_count: 2,
                },
                DeclaredHandlerPlanEntry {
                    kind: DeclaredHandlerKind::Work,
                    runtime_registration_name: "Work.handle_retry".to_string(),
                    executable_symbol: "handle_retry".to_string(),
                    replication_count: 3,
                },
            ],
        )
        .expect("declared work helpers should wrap cleanly");

        assert_eq!(
            registrations,
            vec![
                DeclaredRuntimeRegistration {
                    runtime_registration_name: "Work.handle_submit".to_string(),
                    executable_symbol: "__declared_work_work_handle_submit".to_string(),
                    replication_count: 2,
                },
                DeclaredRuntimeRegistration {
                    runtime_registration_name: "Work.handle_retry".to_string(),
                    executable_symbol: "__declared_work_work_handle_retry".to_string(),
                    replication_count: 3,
                },
            ]
        );
        assert!(mir.functions.iter().any(|func| {
            func.name == "__declared_work_work_handle_submit"
                && func.params == vec![("__args_ptr".to_string(), MirType::Ptr)]
                && func.return_type == MirType::Unit
        }));
        assert!(mir.functions.iter().any(|func| {
            func.name == "__declared_work_work_handle_retry"
                && func.params == vec![("__args_ptr".to_string(), MirType::Ptr)]
                && func.return_type == MirType::Unit
        }));
    }

    #[test]
    fn declared_runtime_handlers_wrap_zero_arg_work_with_hidden_metadata() {
        let mut mir = empty_module();
        mir.functions.push(MirFunction {
            name: "add".to_string(),
            params: vec![],
            return_type: MirType::Int,
            body: MirExpr::IntLit(2, MirType::Int),
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls: false,
        });

        let registrations = prepare_declared_runtime_handlers(
            &mut mir,
            &[DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Work,
                runtime_registration_name: "Work.add".to_string(),
                executable_symbol: "add".to_string(),
                replication_count: 2,
            }],
        )
        .expect("zero-arg declared work should wrap cleanly");

        assert_eq!(
            registrations,
            vec![DeclaredRuntimeRegistration {
                runtime_registration_name: "Work.add".to_string(),
                executable_symbol: "__declared_work_work_add".to_string(),
                replication_count: 2,
            }]
        );
        assert!(mir.functions.iter().any(|func| {
            func.name == "__actor___declared_work_work_add_body"
                && func.params
                    == vec![
                        ("request_key".to_string(), MirType::String),
                        ("attempt_id".to_string(), MirType::String),
                    ]
                && func.return_type == MirType::Unit
        }));
    }

    #[test]
    fn declared_runtime_handlers_reject_public_continuity_parameters() {
        let mut mir = empty_module();
        mir.functions.push(MirFunction {
            name: "add".to_string(),
            params: vec![
                ("request_key".to_string(), MirType::String),
                ("attempt_id".to_string(), MirType::String),
            ],
            return_type: MirType::Int,
            body: MirExpr::IntLit(2, MirType::Int),
            is_closure_fn: false,
            captures: Vec::new(),
            has_tail_calls: false,
        });

        let error = prepare_declared_runtime_handlers(
            &mut mir,
            &[DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Work,
                runtime_registration_name: "Work.add".to_string(),
                executable_symbol: "add".to_string(),
                replication_count: 2,
            }],
        )
        .expect_err("public continuity parameters must be rejected");

        assert!(
            error.contains("continuity metadata is runtime-owned"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn declared_runtime_handlers_reject_missing_lowered_symbol_before_registration() {
        let mut mir = empty_module();

        let error = prepare_declared_runtime_handlers(
            &mut mir,
            &[DeclaredHandlerPlanEntry {
                kind: DeclaredHandlerKind::Work,
                runtime_registration_name: "Work.handle_submit".to_string(),
                executable_symbol: "missing_handle_submit".to_string(),
                replication_count: 2,
            }],
        )
        .expect_err("missing lowered work symbol must fail before registration");

        assert!(
            error.contains(
                "declared work target `Work.handle_submit` has no lowered function `missing_handle_submit`"
            ),
            "unexpected error: {error}"
        );
    }
}
