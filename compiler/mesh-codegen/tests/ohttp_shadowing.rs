use mesh_codegen::mir::{lower::lower_to_mir, MirExpr};
use mesh_typeck::{ImportContext, ModuleExports};
use std::collections::{HashMap, HashSet};
#[test]
fn imported_flat_lookalike_does_not_change_the_real_qualified_ohttp_runtime_borrow() {
    let dependency=mesh_parser::parse("pub fn ohttp_decapsulate_request(key :: X25519PrivateKey,id :: Int,wire :: Bytes) -> OhttpRequestOpening!CryptoError do Secret.destroy(key)\nErr(InvalidKey) end");
    assert!(dependency.ok(), "{:?}", dependency.errors());
    let checked = mesh_typeck::check(&dependency);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &checked);
    let mut imports = ImportContext::empty();
    imports.module_exports.insert(
        "Untrusted.Provider".into(),
        ModuleExports::new("Untrusted.Provider".into(), &exports),
    );
    let source=mesh_parser::parse("from Untrusted.Provider import ohttp_decapsulate_request\nfn genuine(key :: borrow X25519PrivateKey,wire :: Bytes) -> OhttpRequestOpening!CryptoError do Ohttp.decapsulate_request(key,37,wire) end");
    assert!(source.ok(), "{:?}", source.errors());
    let checked = mesh_typeck::check_with_imports(&source, &imports);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let mir = lower_to_mir(&source, &checked, "", &HashSet::new(), &HashMap::new()).unwrap();
    let body = &mir
        .functions
        .iter()
        .find(|f| f.name == "genuine")
        .unwrap()
        .body;
    let args=body.descendants().into_iter().find_map(|expr| match expr { MirExpr::Call {func,args,..} if matches!(func.as_ref(),MirExpr::Var(name,_) if name=="mesh_ohttp_decapsulate_request")=>Some(args),_=>None }).expect("actual builtin call");
    assert!(
        matches!(args[0], MirExpr::ResourceBorrow { .. }),
        "user import changed runtime key ABI ownership: {body:?}"
    );
}
#[test]
fn local_flat_lookalike_does_not_change_the_real_qualified_ohttp_runtime_borrow() {
    let source=mesh_parser::parse("fn ohttp_decapsulate_request(key :: X25519PrivateKey,id :: Int,wire :: Bytes) -> OhttpRequestOpening!CryptoError do Secret.destroy(key)\nErr(InvalidKey) end\nfn genuine(key :: borrow X25519PrivateKey,wire :: Bytes) -> OhttpRequestOpening!CryptoError do Ohttp.decapsulate_request(key,37,wire) end");
    assert!(source.ok(), "{:?}", source.errors());
    let checked = mesh_typeck::check(&source);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let mir = lower_to_mir(&source, &checked, "", &HashSet::new(), &HashMap::new()).unwrap();
    let body = &mir
        .functions
        .iter()
        .find(|f| f.name == "genuine")
        .unwrap()
        .body;
    let args=body.descendants().into_iter().find_map(|expr| match expr { MirExpr::Call {func,args,..} if matches!(func.as_ref(),MirExpr::Var(name,_) if name=="mesh_ohttp_decapsulate_request")=>Some(args),_=>None }).expect("actual builtin call");
    assert!(
        matches!(args[0], MirExpr::ResourceBorrow { .. }),
        "user definition changed runtime key ABI ownership: {body:?}"
    );
}
