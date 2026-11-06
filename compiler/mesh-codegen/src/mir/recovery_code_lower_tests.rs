//! Recovery generation/import ABI and resource lifetime; shadowing stays ordinary code.
use super::lower::*;
use super::*;
use std::collections::{HashMap, HashSet};

fn lower(source: &str) -> MirModule {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    let checked = mesh_typeck::check(&parsed);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    lower_to_mir(&parsed, &checked, "", &HashSet::new(), &HashMap::new()).unwrap()
}
fn has_call(module: &MirModule, symbol: &str, argc: usize) -> bool {
    module.functions.iter().any(|f| f.body.descendants().into_iter().any(|e|
        matches!(e, MirExpr::Call{func,args,..} if matches!(func.as_ref(),MirExpr::Var(n,_) if n==symbol) && args.len()==argc)))
}
#[test]
fn recovery_code_lower_both_public_spellings_have_exact_runtime_calls() {
    for prefix in ["RecoveryCode.", "recovery_code_"] {
        let module=lower(&format!("fn create() do {prefix}generate() end\nfn restore(code :: String) do {prefix}parse(code) end"));
        assert!(has_call(&module, "mesh_recovery_code_generate", 0));
        assert!(has_call(&module, "mesh_recovery_code_parse", 1));
    }
}
#[test]
fn recovery_code_lower_tuple_drop_only_destroys_opaque_secret_field() {
    let module = lower("fn discard(pair :: (SecretBytes,String)) do nil end");
    let drop = module
        .functions
        .iter()
        .find(|f| f.name == "discard")
        .unwrap()
        .body
        .descendants()
        .into_iter()
        .find_map(|e| {
            if let MirExpr::ResourceDrop { destructor, .. } = e {
                Some(destructor)
            } else {
                None
            }
        })
        .unwrap();
    let fields = drop.fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].index, 0);
    assert!(matches!(
        fields[0].destructor,
        MirResourceDestructor::Opaque
    ));
}
#[test]
fn recovery_code_types_reject_wrong_argument_and_generated_secret_reuse() {
    for source in [
        "fn bad() do RecoveryCode.parse(1) end",
        "fn bad() do RecoveryCode.generate(1) end",
        "fn bad() do case RecoveryCode.generate() do Err(_) -> nil Ok((key,code)) ->\nSecret.destroy(key)\nSecret.destroy(key)\nend\nend",
    ] {
        let parsed=mesh_parser::parse(source);assert!(parsed.ok(),"{:?}",parsed.errors());
        let checked=mesh_typeck::check(&parsed);assert!(!checked.errors.is_empty(),"invalid public operation accepted");
    }
}
#[test]
fn recovery_code_import_and_local_shadowing_keep_user_functions() {
    let module=lower("fn recovery_code_parse(value :: Int) -> Int do value + 1 end\nfn use() do recovery_code_parse(2) end");
    assert!(!has_call(&module, "mesh_recovery_code_parse", 1));
    let definition = mesh_parser::parse("pub fn parse(value :: Int) -> Int do value + 1 end");
    let checked = mesh_typeck::check(&definition);
    assert!(checked.errors.is_empty());
    let exports = mesh_typeck::collect_exports(&definition, &checked);
    let mut imports = mesh_typeck::ImportContext::empty();
    imports.module_exports.insert(
        "Foreign.RecoveryCode".into(),
        mesh_typeck::ModuleExports::new("Foreign.RecoveryCode".into(), &exports),
    );
    let parsed =
        mesh_parser::parse("import Foreign.RecoveryCode\nfn use() do RecoveryCode.parse(2) end");
    let checked = mesh_typeck::check_with_imports(&parsed, &imports);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let module = lower_to_mir(&parsed, &checked, "", &HashSet::new(), &HashMap::new()).unwrap();
    assert!(!has_call(&module, "mesh_recovery_code_parse", 1));
}
#[test]
fn recovery_code_intrinsics_use_exact_pointer_abi() {
    let context = inkwell::context::Context::create();
    let module = context.create_module("recovery-code-abi");
    crate::codegen::intrinsics::declare_intrinsics(&module);
    for (name, argc) in [
        ("mesh_recovery_code_generate", 0),
        ("mesh_recovery_code_parse", 1),
    ] {
        let function = module.get_function(name).unwrap();
        let signature = function.get_type();
        assert!(signature.get_return_type().unwrap().is_pointer_type());
        assert_eq!(signature.count_param_types() as usize, argc);
        assert!(signature
            .get_param_types()
            .iter()
            .all(|t| t.is_pointer_type()));
    }
}
