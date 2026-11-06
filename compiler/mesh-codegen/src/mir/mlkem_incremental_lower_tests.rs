use super::lower::*;
use super::*;
use std::collections::{HashMap, HashSet};
fn lower(source: &str) -> MirModule {
    let p = mesh_parser::parse(source);
    assert!(p.ok(), "{:?}", p.errors());
    let t = mesh_typeck::check(&p);
    assert!(t.errors.is_empty(), "{:?}", t.errors);
    lower_to_mir(&p, &t, "", &HashSet::new(), &HashMap::new()).unwrap()
}
fn calls(body: &MirExpr, name: &str) -> bool {
    body.descendants().iter().any(|e|matches!(e,MirExpr::Call{func,..}if matches!(func.as_ref(),MirExpr::Var(n,_)if n==name)))
}
#[test]
fn c1_mir_maps_six_genuine_symbols_and_wrapper_layouts() {
    let m = lower(
        r#"
fn parts(key :: borrow MlKemPrivateKey) do Crypto.mlkem_public_parts(key) end
fn first(header :: MlKemHeader64) do Crypto.mlkem_encapsulate1(header) end
fn finish(state :: borrow MlKemEncapsulationState,vector :: MlKemPublicVector1152) do Crypto.mlkem_encapsulate2(state,vector) end
fn open(key :: borrow MlKemPrivateKey,one :: MlKemCiphertext1_960,two :: MlKemCiphertext2_128) do Crypto.mlkem_decapsulate_parts(key,one,two) end
fn save(state :: borrow MlKemEncapsulationState,key :: borrow StorageKey,context :: Bytes) do MlKemEncapsulationState.seal_for_storage(state,key,context) end
fn load(blob :: Bytes,key :: borrow StorageKey,context :: Bytes) do MlKemEncapsulationState.unseal_from_storage(blob,key,context) end
"#,
    );
    for (function, symbol, arity) in [
        ("parts", "mesh_crypto_mlkem_public_parts", 1),
        ("first", "mesh_crypto_mlkem_encapsulate1", 1),
        ("finish", "mesh_crypto_mlkem_encapsulate2", 2),
        ("open", "mesh_crypto_mlkem_decapsulate_parts", 3),
        ("save", "mesh_mlkem_encapsulation_state_seal_for_storage", 3),
        (
            "load",
            "mesh_mlkem_encapsulation_state_unseal_from_storage",
            3,
        ),
    ] {
        let f = m.functions.iter().find(|f| f.name == function).unwrap();
        assert!(calls(&f.body, symbol), "{f:?}");
        let c=f.body.descendants().into_iter().find(|e|matches!(e,MirExpr::Call{func,..}if matches!(func.as_ref(),MirExpr::Var(n,_)if n==symbol))).unwrap();
        if let MirExpr::Call { args, .. } = c {
            assert_eq!(args.len(), arity);
            if matches!(function, "parts" | "finish" | "open" | "save") {
                assert!(
                    matches!(args[0], MirExpr::ResourceBorrow { .. }),
                    "{function}: {:?}",
                    args[0]
                );
            }
        }
    }
    for name in [
        "MlKemHeader64",
        "MlKemPublicVector1152",
        "MlKemCiphertext1_960",
        "MlKemCiphertext2_128",
    ] {
        assert_eq!(
            m.structs.iter().find(|s| s.name == name).unwrap().fields,
            vec![("bytes".into(), MirType::Ptr)]
        );
    }
}
#[test]
fn c1_mir_opaque_state_and_nested_tuple_drop_use_generic_resource_destructor() {
    let m=lower("fn implicit(state :: MlKemEncapsulationState) do nil end\nfn aggregate(value :: (MlKemEncapsulationState,MlKemCiphertext1_960,SecretBytes)) do nil end");
    for n in ["implicit", "aggregate"] {
        let b = &m.functions.iter().find(|f| f.name == n).unwrap().body;
        assert!(
            b.descendants()
                .iter()
                .any(|e| matches!(e, MirExpr::ResourceDrop { .. })),
            "{b:?}"
        );
        let d = b
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
        if n == "implicit" {
            assert!(matches!(d, MirResourceDestructor::Opaque));
        } else {
            let fields = d.fields();
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].index, 0);
            assert_eq!(fields[1].index, 2);
            assert!(fields
                .iter()
                .all(|f| matches!(f.destructor, MirResourceDestructor::Opaque)));
        }
    }
}
#[test]
fn c1_mir_user_crypto_names_are_not_rewritten_to_runtime_symbols() {
    let m = lower("fn crypto_mlkem_encapsulate2(a :: Int,b :: Int) -> Int do a+b end\nfn use() -> Int do crypto_mlkem_encapsulate2(1,2) end");
    let f = m.functions.iter().find(|f| f.name == "use").unwrap();
    assert!(!calls(&f.body, "mesh_crypto_mlkem_encapsulate2"), "{f:?}");
    let dependency=mesh_parser::parse("pub fn mlkem_encapsulate2(state :: borrow MlKemEncapsulationState,vector :: MlKemPublicVector1152) -> MlKemCiphertext2_128!CryptoError do Ok(MlKemCiphertext2_128 { bytes: Bytes.empty() }) end");
    let checked = mesh_typeck::check(&dependency);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &checked);
    let mut imports = mesh_typeck::ImportContext::empty();
    imports.module_exports.insert(
        "Foreign.Crypto".into(),
        mesh_typeck::ModuleExports::new("Foreign.Crypto".into(), &exports),
    );
    let p=mesh_parser::parse("import Foreign.Crypto\nfn use(state :: borrow MlKemEncapsulationState,vector :: MlKemPublicVector1152) do Crypto.mlkem_encapsulate2(state,vector) end");
    assert!(p.ok(), "{:?}", p.errors());
    let t = mesh_typeck::check_with_imports(&p, &imports);
    assert!(t.errors.is_empty(), "{:?}", t.errors);
    assert!(
        t.qualified_modules
            .get("Crypto")
            .is_some_and(|f| f.contains(&"mlkem_encapsulate2".to_string())),
        "{:?}",
        t.qualified_modules
    );
    let m = lower_to_mir(&p, &t, "", &HashSet::new(), &HashMap::new()).unwrap();
    let f = m.functions.iter().find(|f| f.name == "use").unwrap();
    assert!(!calls(&f.body, "mesh_crypto_mlkem_encapsulate2"), "{f:?}");
    assert!(
        m.functions
            .iter()
            .any(|f| calls(&f.body, "mlkem_encapsulate2")),
        "{m:?}"
    );
}

#[test]
fn c1_public_validation_mir_genuine_calls_have_two_ordinary_wrappers() {
    let m=lower("fn qualified(h :: MlKemHeader64,v :: MlKemPublicVector1152) -> Result<(),CryptoError> do Crypto.mlkem_validate_public_parts(h,v) end\nfn flat(h :: MlKemHeader64,v :: MlKemPublicVector1152) do crypto_mlkem_validate_public_parts(h,v) end");
    for name in ["qualified", "flat"] {
        let f = m.functions.iter().find(|f| f.name == name).unwrap();
        assert!(
            calls(&f.body, "mesh_crypto_mlkem_validate_public_parts"),
            "{f:?}"
        );
        for node in f.body.descendants() {
            assert!(
                !matches!(
                    node,
                    MirExpr::ResourceMove { .. }
                        | MirExpr::ResourceBorrow { .. }
                        | MirExpr::ResourceDrop { .. }
                ),
                "public validation gained secret ownership: {node:?}"
            );
        }
    }
    let fake=lower("fn crypto_mlkem_validate_public_parts(a :: Int,b :: Int) -> Int do a+b end\nfn local() -> Int do crypto_mlkem_validate_public_parts(2,3) end");
    assert!(!calls(
        &fake
            .functions
            .iter()
            .find(|f| f.name == "local")
            .unwrap()
            .body,
        "mesh_crypto_mlkem_validate_public_parts"
    ));

    let dependency = mesh_parser::parse(
        "pub fn mlkem_validate_public_parts(a :: Int,b :: Int) -> Int do a+b end",
    );
    let checked = mesh_typeck::check(&dependency);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &checked);
    let mut imports = mesh_typeck::ImportContext::empty();
    imports.module_exports.insert(
        "Foreign.Crypto".into(),
        mesh_typeck::ModuleExports::new("Foreign.Crypto".into(), &exports),
    );
    let parsed = mesh_parser::parse("import Foreign.Crypto\nfn imported() -> Int do Crypto.mlkem_validate_public_parts(3,4) end");
    let checked = mesh_typeck::check_with_imports(&parsed, &imports);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let imported = lower_to_mir(&parsed, &checked, "", &HashSet::new(), &HashMap::new()).unwrap();
    assert!(!calls(
        &imported
            .functions
            .iter()
            .find(|f| f.name == "imported")
            .unwrap()
            .body,
        "mesh_crypto_mlkem_validate_public_parts"
    ));
}
