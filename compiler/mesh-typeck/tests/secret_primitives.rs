//! API availability is checked before negative ownership cases.
use mesh_typeck::{error::TypeError, TypeckResult};
fn check(source: &str) -> TypeckResult {
    let p = mesh_parser::parse(source);
    assert!(p.ok(), "{:?}", p.errors());
    mesh_typeck::check(&p)
}
fn good(source: &str) {
    let t = check(source);
    assert!(t.errors.is_empty(), "{source}\n{:?}", t.errors);
}
const API: &str = r#"
fn derive(ikm :: borrow SecretBytes,salt :: borrow SecretBytes,info :: Bytes,n :: Int) -> SecretBytes!CryptoError do Crypto.hkdf_sha256_secret_salt(ikm,salt,info,n) end
fn split(value :: consume SecretBytes,n :: Int) -> (SecretBytes,SecretBytes)!CryptoError do Secret.split_at(value,n) end
fn tag(key :: borrow SecretBytes,message :: Bytes) -> Bytes!CryptoError do Crypto.hmac_sha256_tag(key,message) end
"#;
#[test]
fn secret_primitives_three_exact_signatures_and_aliases() {
    good(API);
    good(
        r#"
fn derive(ikm :: borrow SecretBytes,salt :: borrow SecretBytes,info :: Bytes) do crypto_hkdf_sha256_secret_salt(ikm,salt,info,64) end
fn again(ikm :: borrow SecretBytes,salt :: borrow SecretBytes) -> Bytes!CryptoError do
 let first=Crypto.hkdf_sha256_secret_salt(ikm,salt,Bytes.empty(),32)?
 let second=ikm |> Crypto.hkdf_sha256_secret_salt(salt,Bytes.empty(),32)?
 let one=Crypto.hmac_sha256_tag(first,Bytes.empty())?
 let _two=crypto_hmac_sha256_tag(second,Bytes.empty())?
 Secret.destroy(first)
 Secret.destroy(second)
 Crypto.hmac_sha256_tag(ikm,one)
end
fn split(value :: consume SecretBytes) do value |> Secret.split_at(32) end
fn split_alias(value :: consume SecretBytes) do secret_split_at(value,32) end
fn same_salt(value :: borrow SecretBytes) do Crypto.hkdf_sha256_secret_salt(value,value,Bytes.empty(),32) end
"#,
    );
}
#[test]
fn secret_primitives_split_moves_whole_result_tuple_and_each_member() {
    good(API);
    for s in [
 "fn bad(value :: SecretBytes) do let pending=Secret.split_at(value,1)\nvalue end",
 "fn bad(value :: SecretBytes) do let pending=Secret.split_at(value,1)\nlet moved=pending\npending end",
 "fn bad(value :: SecretBytes) -> SecretBytes!CryptoError do let pair=Secret.split_at(value,1)?\nlet moved=pair\nlet (a,b)=pair\nOk(a) end",
 "fn bad(value :: SecretBytes) -> SecretBytes!CryptoError do let pair=Secret.split_at(value,1)?\nlet (a,b)=pair\nlet copy=a\nOk(a) end",
 "fn bad(value :: borrow SecretBytes) do Secret.split_at(value,1) end",
 "fn bad(value :: SecretBytes) -> (SecretBytes,SecretBytes)!CryptoError do let (a,b)=Secret.split_at(value,1)?\nOk((a,a)) end",
 ] {let t=check(s);assert!(t.errors.iter().any(|e|matches!(e,TypeError::ResourceViolation{..})),"wrong refusal {s}: {:?}",t.errors);assert!(t.errors.iter().all(|e|matches!(e,TypeError::ResourceViolation{..})),"unrelated error in ownership negative {s}: {:?}",t.errors);}
    good("fn drop_pair(value :: (SecretBytes,SecretBytes)) do nil end");
}
#[test]
fn secret_primitives_borrowed_inputs_cannot_escape_or_be_indirectly_reclassified() {
    good(API);
    for s in [
 "fn bad(value :: borrow SecretBytes) -> SecretBytes do value end",
 "fn bad(a :: borrow SecretBytes,b :: borrow SecretBytes) do let f=Crypto.hkdf_sha256_secret_salt\nf(a,b,Bytes.empty(),32) end",
 "fn bad(key :: borrow SecretBytes) do let f=Crypto.hmac_sha256_tag\nf(key,Bytes.empty()) end",
 "fn bad(value :: SecretBytes) do let f=Secret.split_at\nf(value,1) end",
 ] {let t=check(s);assert!(t.errors.iter().any(|e|matches!(e,TypeError::ResourceViolation{..})),"{s}: {:?}",t.errors);assert!(t.errors.iter().all(|e|matches!(e,TypeError::ResourceViolation{..})),"unrelated error in borrowed/indirect negative {s}: {:?}",t.errors);}
}
#[test]
fn secret_primitives_have_no_secret_byte_export_or_bytes_to_secret_coercion() {
    good(API);
    for s in [
 "fn bad(value :: SecretBytes) do value.bytes end",
 "fn bad(value :: SecretBytes) do Bytes.to_hex(value) end",
 "fn bad(value :: SecretBytes) do Bytes.slice(value,0,1) end",
 "fn bad(value :: Bytes) do Secret.split_at(value,1) end",
 "fn bad(a :: borrow SecretBytes,b :: Bytes) do Crypto.hkdf_sha256_secret_salt(a,b,Bytes.empty(),32) end",
 "fn bad(value :: borrow AeadKey) do Crypto.hmac_sha256_tag(value,Bytes.empty()) end",
 ] {
 let t=check(s);assert!(!t.errors.is_empty(),"accepted {s}");
 assert!(t.errors.iter().all(|e|matches!(e,TypeError::Mismatch{..}|TypeError::NoSuchField{..}|TypeError::ResourceViolation{..})),"unrelated error in no-export/type negative {s}: {:?}",t.errors);
 assert!(t.errors.iter().any(|e|matches!(e,TypeError::Mismatch{..}|TypeError::NoSuchField{..})),"missing actual type/field refusal {s}: {:?}",t.errors);
}
    good("fn old(a :: borrow SecretBytes,salt :: Bytes) -> SecretBytes!CryptoError do Crypto.hkdf_sha256(a,salt,Bytes.empty(),32) end\nfn old_mac(a :: borrow SecretBytes) -> SecretBytes!CryptoError do Crypto.hmac_sha256(a,Bytes.empty()) end");
}
#[test]
fn secret_primitives_user_and_imported_lookalikes_remain_ordinary_functions() {
    good(API);
    good("fn secret_split_at(a :: Int,b :: Int) -> Int do a+b end\nfn use() -> Int do secret_split_at(1,2) end");
    let dep = mesh_parser::parse("pub fn hmac_sha256_tag(a :: Int,b :: Int) -> Int do a+b end");
    let checked = mesh_typeck::check(&dep);
    assert!(checked.errors.is_empty());
    let exports = mesh_typeck::collect_exports(&dep, &checked);
    let mut imports = mesh_typeck::ImportContext::empty();
    imports.module_exports.insert(
        "Foreign.Crypto".into(),
        mesh_typeck::ModuleExports::new("Foreign.Crypto".into(), &exports),
    );
    let p = mesh_parser::parse(
        "import Foreign.Crypto\nfn use() -> Int do Crypto.hmac_sha256_tag(1,2) end",
    );
    let t = mesh_typeck::check_with_imports(&p, &imports);
    assert!(t.errors.is_empty(), "{:?}", t.errors);
    assert!(t
        .qualified_modules
        .get("Crypto")
        .is_some_and(|v| v.contains(&"hmac_sha256_tag".to_string())));
}
