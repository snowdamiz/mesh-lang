//! Monomorphization pass.
//!
//! Takes a MIR module and ensures all functions use only concrete types.
//! Since the type checker already resolves concrete types at each call site,
//! this pass primarily:
//! 1. Collects all reachable functions starting from the entry point.
//! 2. Removes unreachable functions.
//! 3. In future: creates specialized copies of generic functions for each
//!    concrete type instantiation.
//!
//! For Phase 5, all types are already concrete after lowering (the type checker
//! resolves all generics), so monomorphization is mainly a reachability pass.

use std::collections::HashSet;

use super::{MirExpr, MirModule};

/// Run the monomorphization pass on a MIR module.
///
/// This collects all reachable functions starting from the entry point
/// (or all top-level functions if no entry point exists), plus any explicit
/// extra roots that the caller needs to preserve, and removes any unreachable
/// functions. In the future, this will also specialize generic functions for each
/// concrete type instantiation.
pub fn monomorphize(module: &mut MirModule) {
    monomorphize_with_roots(module, &[]);
}

/// Run the monomorphization pass while preserving additional root symbols.
pub fn monomorphize_with_roots(module: &mut MirModule, extra_roots: &[String]) {
    let reachable = collect_reachable_functions(module, extra_roots);

    // Keep only reachable functions (plus closure functions that may be
    // referenced transitively).
    module.functions.retain(|f| reachable.contains(&f.name));
    module
        .native_functions
        .retain(|function| reachable.contains(&function.name));
}

/// Collect the names of all reachable functions starting from the entry point.
fn collect_reachable_functions(module: &MirModule, extra_roots: &[String]) -> HashSet<String> {
    let mut reachable = HashSet::new();
    let mut worklist: Vec<String> = Vec::new();

    // Start from the entry function, or all functions if no entry.
    if let Some(ref entry) = module.entry_function {
        worklist.push(entry.clone());
    } else if extra_roots.is_empty() {
        // No entry point: keep all functions reachable.
        for f in &module.functions {
            worklist.push(f.name.clone());
        }
        for function in &module.native_functions {
            worklist.push(function.name.clone());
        }
    }
    for root in extra_roots {
        worklist.push(root.clone());
    }

    while let Some(name) = worklist.pop() {
        if reachable.contains(&name) {
            continue;
        }
        reachable.insert(name.clone());

        // If this is a service loop function, add all handler functions from
        // the dispatch table as reachable. The loop body is MirExpr::Unit
        // (codegen generates the dispatch inline), so handler functions are
        // not referenced from MIR expressions -- only from the dispatch table.
        if let Some((call_handlers, cast_handlers)) = module.service_dispatch.get(&name) {
            for (_, handler_fn, _) in call_handlers {
                if !reachable.contains(handler_fn) {
                    worklist.push(handler_fn.clone());
                }
            }
            for (_, handler_fn, _) in cast_handlers {
                if !reachable.contains(handler_fn) {
                    worklist.push(handler_fn.clone());
                }
            }
        }

        // If this is an actor wrapper function, add the body function as reachable.
        // Actor wrappers have body: MirExpr::Unit and a single __args_ptr param.
        // The body function is named __actor_{name}_body and is called by codegen,
        // not referenced in MIR expressions.
        if let Some(func) = module.functions.iter().find(|f| f.name == name) {
            if func.params.len() == 1 && func.params[0].0 == "__args_ptr" {
                let body_fn_name = format!("__actor_{}_body", name);
                if module.functions.iter().any(|f| f.name == body_fn_name)
                    && !reachable.contains(&body_fn_name)
                {
                    worklist.push(body_fn_name);
                }
            }
        }

        // Find the function and scan its body for referenced functions.
        if let Some(func) = module.functions.iter().find(|f| f.name == name) {
            let mut refs = Vec::new();
            collect_function_refs(&func.body, &mut refs);
            for r in refs {
                if !reachable.contains(&r) {
                    worklist.push(r);
                }
            }
        }
    }

    reachable
}

#[cfg(test)]
mod library_tests {
    use super::*;
    use crate::mir::{MirFunction, MirType};

    fn function(name: &str) -> MirFunction {
        MirFunction {
            name: name.to_string(),
            params: vec![],
            return_type: MirType::Unit,
            body: MirExpr::Unit,
            is_closure_fn: false,
            captures: vec![],
            has_tail_calls: false,
        }
    }

    #[test]
    fn explicit_library_roots_prune_unrelated_functions_without_main() {
        let mut module = MirModule {
            functions: vec![function("exported"), function("unrelated")],
            native_functions: vec![],
            structs: vec![],
            sum_types: vec![],
            entry_function: None,
            service_dispatch: std::collections::HashMap::new(),
        };
        monomorphize_with_roots(&mut module, &["exported".to_string()]);
        assert_eq!(
            module
                .functions
                .iter()
                .map(|function| function.name.as_str())
                .collect::<Vec<_>>(),
            ["exported"]
        );
    }
}

/// The function names an expression refers to: every variable (a callee
/// included), each closure's function, a supervisor's child start functions
/// and a `for` loop's iterator functions.
fn collect_function_refs(expr: &MirExpr, refs: &mut Vec<String>) {
    for node in expr.descendants() {
        match node {
            MirExpr::Var(name, _) => refs.push(name.clone()),
            MirExpr::MakeClosure { fn_name, .. } => refs.push(fn_name.clone()),
            MirExpr::SupervisorStart { children, .. } => refs.extend(
                children
                    .iter()
                    .filter(|child| !child.start_fn.is_empty())
                    .map(|child| child.start_fn.clone()),
            ),
            MirExpr::ForInIterator {
                next_fn, iter_fn, ..
            } => {
                refs.push(next_fn.clone());
                if !iter_fn.is_empty() {
                    refs.push(iter_fn.clone());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{MirFunction, MirType};

    #[test]
    fn monomorphize_keeps_reachable_functions() {
        let mut module = MirModule {
            functions: vec![
                MirFunction {
                    name: "main".to_string(),
                    params: vec![],
                    return_type: MirType::Int,
                    body: MirExpr::Call {
                        func: Box::new(MirExpr::Var(
                            "helper".to_string(),
                            MirType::FnPtr(vec![], Box::new(MirType::Int)),
                        )),
                        args: vec![],
                        ty: MirType::Int,
                    },
                    is_closure_fn: false,
                    captures: vec![],
                    has_tail_calls: false,
                },
                MirFunction {
                    name: "helper".to_string(),
                    params: vec![],
                    return_type: MirType::Int,
                    body: MirExpr::IntLit(42, MirType::Int),
                    is_closure_fn: false,
                    captures: vec![],
                    has_tail_calls: false,
                },
                MirFunction {
                    name: "unused".to_string(),
                    params: vec![],
                    return_type: MirType::Int,
                    body: MirExpr::IntLit(0, MirType::Int),
                    is_closure_fn: false,
                    captures: vec![],
                    has_tail_calls: false,
                },
            ],
            structs: vec![],
            sum_types: vec![],
            entry_function: Some("main".to_string()),
            service_dispatch: std::collections::HashMap::new(),
            native_functions: vec![],
        };

        monomorphize(&mut module);

        let names: Vec<&str> = module.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(names.contains(&"helper"));
        assert!(
            !names.contains(&"unused"),
            "unused function should be removed"
        );
    }

    #[test]
    fn monomorphize_keeps_all_without_entry() {
        let mut module = MirModule {
            functions: vec![
                MirFunction {
                    name: "foo".to_string(),
                    params: vec![],
                    return_type: MirType::Unit,
                    body: MirExpr::Unit,
                    is_closure_fn: false,
                    captures: vec![],
                    has_tail_calls: false,
                },
                MirFunction {
                    name: "bar".to_string(),
                    params: vec![],
                    return_type: MirType::Unit,
                    body: MirExpr::Unit,
                    is_closure_fn: false,
                    captures: vec![],
                    has_tail_calls: false,
                },
            ],
            structs: vec![],
            sum_types: vec![],
            entry_function: None,
            service_dispatch: std::collections::HashMap::new(),
            native_functions: vec![],
        };

        monomorphize(&mut module);

        assert_eq!(module.functions.len(), 2);
    }
}
