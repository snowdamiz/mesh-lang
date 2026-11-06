//! Genuine builtin mappings and exact borrow/move semantics.
use super::lower::*;
use super::*;
use std::collections::{HashMap, HashSet};
fn lower(s: &str) -> MirModule {
    let p = mesh_parser::parse(s);
    assert!(p.ok(), "{:?}", p.errors());
    let t = mesh_typeck::check(&p);
    assert!(t.errors.is_empty(), "{:?}", t.errors);
    lower_to_mir(&p, &t, "", &HashSet::new(), &HashMap::new()).unwrap()
}
fn call<'a>(m: &'a MirModule, function: &str, symbol: &str) -> &'a Vec<MirExpr> {
    m.functions.iter().find(|f|f.name==function).unwrap().body.descendants().into_iter().find_map(|e|match e{MirExpr::Call{func,args,..} if matches!(func.as_ref(),MirExpr::Var(n,_)if n==symbol)=>Some(args),_=>None}).unwrap()
}
#[test]
fn secret_primitives_mir_three_exact_symbols_both_hkdf_borrows_and_split_move() {
    for (crypto, secret) in [("Crypto.", "Secret."), ("crypto_", "secret_")] {
        let source=format!("fn derive(a :: borrow SecretBytes,b :: borrow SecretBytes,info :: Bytes,n :: Int) do {crypto}hkdf_sha256_secret_salt(a,b,info,n) end\nfn split(value :: consume SecretBytes,n :: Int) do {secret}split_at(value,n) end\nfn tag(key :: borrow SecretBytes,msg :: Bytes) do {crypto}hmac_sha256_tag(key,msg) end");
        let m = lower(&source);
        let d = call(&m, "derive", "mesh_crypto_hkdf_sha256_secret_salt");
        assert_eq!(d.len(), 4);
        for index in [0, 1] {
            assert!(
                matches!(d[index], MirExpr::ResourceBorrow { .. }),
                "{crypto}: {:?}",
                d[index]
            );
        }
        let s = call(&m, "split", "mesh_secret_split_at");
        assert_eq!(s.len(), 2);
        assert!(
            matches!(s[0], MirExpr::ResourceMove { .. }),
            "{secret}: missing actual move wrapper: {:?}",
            s[0]
        );
        let t = call(&m, "tag", "mesh_crypto_hmac_sha256_tag");
        assert_eq!(t.len(), 2);
        assert!(
            matches!(t[0], MirExpr::ResourceBorrow { .. }),
            "{crypto}: {:?}",
            t[0]
        );
    }
}
#[test]
fn secret_primitives_mir_whole_pair_implicit_drop_destroys_both_fields() {
    let m = lower("fn discard(pair :: (SecretBytes,SecretBytes)) do nil end");
    let b = &m
        .functions
        .iter()
        .find(|f| f.name == "discard")
        .unwrap()
        .body;
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
    let fields = d.fields();
    assert_eq!(fields.len(), 2);
    assert_eq!((fields[0].index, fields[1].index), (0, 1));
    assert!(fields
        .iter()
        .all(|f| matches!(f.destructor, MirResourceDestructor::Opaque)));
}
#[test]
fn secret_primitives_mir_local_lookalike_names_are_not_intrinsics() {
    let m=lower("fn crypto_hmac_sha256_tag(a :: Int,b :: Int) -> Int do a+b end\nfn secret_split_at(a :: Int,b :: Int) -> Int do a+b end\nfn use() -> Int do secret_split_at(crypto_hmac_sha256_tag(1,2),3) end");
    let b = &m.functions.iter().find(|f| f.name == "use").unwrap().body;
    assert!(!b.descendants().into_iter().any(|e|matches!(e,MirExpr::Call{func,..}if matches!(func.as_ref(),MirExpr::Var(n,_)if n=="mesh_crypto_hmac_sha256_tag"||n=="mesh_secret_split_at"))));
}

#[test]
fn secret_primitives_mir_imported_qualified_lookalikes_do_not_gain_native_symbols() {
    for (module,method,definition,source,symbol) in [
  ("Foreign.Crypto","hmac_sha256_tag","pub fn hmac_sha256_tag(key :: borrow SecretBytes,msg :: Bytes) -> Bytes!CryptoError do Ok(Bytes.empty()) end","import Foreign.Crypto\nfn use(key :: borrow SecretBytes,msg :: Bytes) do Crypto.hmac_sha256_tag(key,msg) end","mesh_crypto_hmac_sha256_tag"),
  ("Foreign.Crypto","hkdf_sha256_secret_salt","pub fn hkdf_sha256_secret_salt(a :: borrow SecretBytes,b :: borrow SecretBytes,info :: Bytes,n :: Int) -> Bytes!CryptoError do Ok(info) end","import Foreign.Crypto\nfn use(a :: borrow SecretBytes,b :: borrow SecretBytes,info :: Bytes) do Crypto.hkdf_sha256_secret_salt(a,b,info,32) end","mesh_crypto_hkdf_sha256_secret_salt"),
  ("Foreign.Secret","split_at","pub fn split_at(value :: consume SecretBytes,n :: Int) -> (SecretBytes,SecretBytes)!CryptoError do Err(InvalidKey) end","import Foreign.Secret\nfn use(value :: consume SecretBytes) do Secret.split_at(value,1) end","mesh_secret_split_at"),
 ]{
  let parsed=mesh_parser::parse(definition);assert!(parsed.ok(),"{:?}",parsed.errors());let checked=mesh_typeck::check(&parsed);assert!(checked.errors.is_empty(),"{:?}",checked.errors);
  let exports=mesh_typeck::collect_exports(&parsed,&checked);let mut imports=mesh_typeck::ImportContext::empty();imports.module_exports.insert(module.into(),mesh_typeck::ModuleExports::new(module.into(),&exports));
  let p=mesh_parser::parse(source);assert!(p.ok(),"{:?}",p.errors());let t=mesh_typeck::check_with_imports(&p,&imports);assert!(t.errors.is_empty(),"{source}: {:?}",t.errors);
  let short=module.rsplit('.').next().unwrap();assert!(t.qualified_modules.get(short).is_some_and(|f|f.contains(&method.to_string())));
  let mir=lower_to_mir(&p,&t,"",&HashSet::new(),&HashMap::new()).unwrap();
  assert!(!mir.functions.iter().any(|f|f.body.descendants().into_iter().any(|e|matches!(e,MirExpr::Call{func,..}if matches!(func.as_ref(),MirExpr::Var(n,_)if n==symbol)))),"imported {module}.{method} was rewritten to {symbol}: {mir:?}");
 }
}

#[test]
fn secret_primitives_intrinsic_declarations_have_exact_pointer_and_i64_abi() {
    let context = inkwell::context::Context::create();
    let module = context.create_module("secret-primitives-abi");
    crate::codegen::intrinsics::declare_intrinsics(&module);
    for (name, integers) in [
        (
            "mesh_crypto_hkdf_sha256_secret_salt",
            vec![false, false, false, true],
        ),
        ("mesh_secret_split_at", vec![false, true]),
        ("mesh_crypto_hmac_sha256_tag", vec![false, false]),
    ] {
        let f = module
            .get_function(name)
            .unwrap_or_else(|| panic!("missing {name}"));
        let params = f.get_type().get_param_types();
        assert_eq!(params.len(), integers.len(), "{name}");
        for (parameter, integer) in params.iter().zip(integers) {
            if integer {
                assert!(parameter.is_int_type(), "{name}");
                assert_eq!(parameter.into_int_type().get_bit_width(), 64, "{name}");
            } else {
                assert!(parameter.is_pointer_type(), "{name}");
            }
        }
        assert!(
            f.get_type().get_return_type().unwrap().is_pointer_type(),
            "{name}"
        );
    }
}
