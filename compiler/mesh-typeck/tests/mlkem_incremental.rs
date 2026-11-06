//! Incremental ML-KEM public type/ownership contracts, no app protocol.
use mesh_typeck::{error::TypeError, TypeckResult};
fn check(source: &str) -> TypeckResult {
    let p = mesh_parser::parse(source);
    assert!(p.ok(), "{:?}", p.errors());
    mesh_typeck::check(&p)
}
fn good(source: &str) -> TypeckResult {
    let r = check(source);
    assert!(r.errors.is_empty(), "{source}\n{:?}", r.errors);
    r
}
const API: &str = r#"
fn parts(key :: borrow MlKemPrivateKey) -> (MlKemHeader64,MlKemPublicVector1152)!CryptoError do Crypto.mlkem_public_parts(key) end
fn first(header :: MlKemHeader64) -> (MlKemEncapsulationState,MlKemCiphertext1_960,SecretBytes)!CryptoError do Crypto.mlkem_encapsulate1(header) end
fn finish(state :: borrow MlKemEncapsulationState, vector :: MlKemPublicVector1152) -> MlKemCiphertext2_128!CryptoError do Crypto.mlkem_encapsulate2(state,vector) end
fn open(key :: borrow MlKemPrivateKey,one :: MlKemCiphertext1_960,two :: MlKemCiphertext2_128) -> SecretBytes!CryptoError do Crypto.mlkem_decapsulate_parts(key,one,two) end
fn save(state :: borrow MlKemEncapsulationState,key :: borrow StorageKey,context :: Bytes) -> Bytes!CryptoError do MlKemEncapsulationState.seal_for_storage(state,key,context) end
fn load(blob :: Bytes,key :: borrow StorageKey,context :: Bytes) -> MlKemEncapsulationState!CryptoError do MlKemEncapsulationState.unseal_from_storage(blob,key,context) end
"#;
#[test]
fn c1_all_six_signatures_wrappers_and_opaque_resource_are_registered() {
    let r = good(API);
    assert!(r.type_registry.is_resource_name("MlKemEncapsulationState"));
    assert!(!r
        .type_registry
        .struct_defs
        .contains_key("MlKemEncapsulationState"));
    for n in [
        "MlKemHeader64",
        "MlKemPublicVector1152",
        "MlKemCiphertext1_960",
        "MlKemCiphertext2_128",
    ] {
        assert!(!r.type_registry.is_resource_name(n));
        let fields = &r.type_registry.struct_defs[n].fields;
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "bytes");
        assert_eq!(fields[0].1, mesh_typeck::ty::Ty::bytes());
    }
}
#[test]
fn c1_prefixed_function_values_and_pipes_preserve_borrows() {
    good(API);
    good(
        r#"
fn parts(key :: borrow MlKemPrivateKey) do crypto_mlkem_public_parts(key) end
fn first(header :: MlKemHeader64) do let op=Crypto.mlkem_encapsulate1
header |> op end
fn twice(state :: borrow MlKemEncapsulationState,vector :: MlKemPublicVector1152,key :: borrow StorageKey,context :: Bytes) -> Bytes!CryptoError do
 let _a=Crypto.mlkem_encapsulate2(state,vector)?
 let _b=crypto_mlkem_encapsulate2(state,vector)?
 let _c=state |> Crypto.mlkem_encapsulate2(vector)?
 let _d=MlKemEncapsulationState.seal_for_storage(state,key,context)?
 mlkem_encapsulation_state_seal_for_storage(state,key,context)
end
fn load(blob :: Bytes,key :: borrow StorageKey,context :: Bytes) do mlkem_encapsulation_state_unseal_from_storage(blob,key,context) end
fn open(key :: borrow MlKemPrivateKey,one :: MlKemCiphertext1_960,two :: MlKemCiphertext2_128) do crypto_mlkem_decapsulate_parts(key,one,two) end
"#,
    );
}
#[test]
fn c1_whole_tuple_projection_and_lifetime_misuse_are_resource_errors() {
    good(API);
    for source in [
 "fn bad(state :: MlKemEncapsulationState) do let moved=state\nstate end",
 "fn bad(state :: MlKemEncapsulationState) do (state,state) end",
 "fn bad(state :: MlKemEncapsulationState) do Secret.destroy(state)\nSecret.destroy(state) end",
 "fn bad(state :: borrow MlKemEncapsulationState) -> MlKemEncapsulationState do state end",
 "fn bad(value :: (MlKemEncapsulationState,MlKemCiphertext1_960,SecretBytes)) do let moved=value\nlet (state,one,shared)=value\nstate end",
 "fn bad(value :: (MlKemEncapsulationState,MlKemCiphertext1_960,SecretBytes)) do let (state,one,shared)=value\nvalue end",
 "fn bad(state :: MlKemEncapsulationState) do let capture=fn() do state end\nlet duplicate=capture\ncapture() end",
 "fn bad(header :: MlKemHeader64) do let pending=Crypto.mlkem_encapsulate1(header)\nlet moved=pending\npending end",
 "fn bad(header :: MlKemHeader64) -> MlKemEncapsulationState!CryptoError do let value=Crypto.mlkem_encapsulate1(header)?\nlet moved=value\nlet (state,one,shared)=value\nOk(state) end",
 ] { let r=check(source);assert!(r.errors.iter().any(|e|matches!(e,TypeError::ResourceViolation{..})),"accepted or wrong failure {source}: {:?}",r.errors); }
}
#[test]
fn c1_opaque_state_has_no_import_export_or_indirect_destroy_coercion() {
    good(API);
    for source in [
 "fn bad(state :: MlKemEncapsulationState) do state.bytes end",
 "fn bad() do MlKemEncapsulationState {} end",
 "fn bad(state :: MlKemEncapsulationState) do Crypto.sha256(state) end",
 "fn bad(state :: MlKemEncapsulationState) do String.from(state) end",
 "fn bad(value :: SecretBytes) -> MlKemEncapsulationState do value end",
 "fn bad(state :: MlKemEncapsulationState) do let destroy=Secret.destroy\ndestroy(state) end",
 "fn bad(key :: borrow MlKemPrivateKey) do Crypto.mlkem_encapsulate1(key) end",
 ] { assert!(!check(source).errors.is_empty(),"accepted {source}"); }
    good("fn drop_state(state :: MlKemEncapsulationState) do Secret.destroy(state) end");
}
#[test]
fn c1_user_names_do_not_gain_builtin_signatures() {
    good(API);
    good("fn crypto_mlkem_encapsulate2(a :: Int,b :: Int) -> Int do a+b end\nfn use() -> Int do crypto_mlkem_encapsulate2(1,2) end");
    let dependency =
        mesh_parser::parse("pub fn mlkem_encapsulate2(a :: Int,b :: Int) -> Int do a+b end");
    let checked = mesh_typeck::check(&dependency);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
    let exports = mesh_typeck::collect_exports(&dependency, &checked);
    let mut imports = mesh_typeck::ImportContext::empty();
    imports.module_exports.insert(
        "Foreign.Crypto".into(),
        mesh_typeck::ModuleExports::new("Foreign.Crypto".into(), &exports),
    );
    let p = mesh_parser::parse(
        "import Foreign.Crypto\nfn use() -> Int do Crypto.mlkem_encapsulate2(1,2) end",
    );
    let r = mesh_typeck::check_with_imports(&p, &imports);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(r
        .qualified_modules
        .get("Crypto")
        .is_some_and(|f| f.contains(&"mlkem_encapsulate2".to_string())));
}

#[test]
fn c1_stored_indirect_state_calls_retain_existing_conservative_refusal() {
    good(API);
    for source in [
 "fn bad(state :: borrow MlKemEncapsulationState,vector :: MlKemPublicVector1152) do let op=Crypto.mlkem_encapsulate2\nop(state,vector) end",
 "fn bad(state :: borrow MlKemEncapsulationState,key :: borrow StorageKey,ctx :: Bytes) do let op=MlKemEncapsulationState.seal_for_storage\nop(state,key,ctx) end"
 ] {let r=check(source);assert!(r.errors.iter().any(|e|matches!(e,TypeError::ResourceViolation{reason,..}if reason.contains("generic or indirect call"))),"{source}: {:?}",r.errors);}
    good("fn finish(state :: borrow MlKemEncapsulationState,vector :: MlKemPublicVector1152) do Crypto.mlkem_encapsulate2(state,vector) end\nfn twice(state :: borrow MlKemEncapsulationState,vector :: MlKemPublicVector1152) -> MlKemCiphertext2_128!CryptoError do let _=finish(state,vector)?\nfinish(state,vector) end");
}
