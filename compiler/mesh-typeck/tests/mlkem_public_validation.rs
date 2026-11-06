//! Synthetic API/type/flow contracts; no app ingress labels or disclosures.
use mesh_typeck::{error::TypeError, TypeckResult};
fn check(source: &str) -> TypeckResult {
    let p = mesh_parser::parse(source);
    assert!(p.ok(), "{source}\n{:?}", p.errors());
    mesh_typeck::check(&p)
}
fn good(source: &str) {
    let result = check(source);
    assert!(result.errors.is_empty(), "{source}\n{:?}", result.errors);
}
const API: &str = "fn validate(h :: MlKemHeader64,v :: MlKemPublicVector1152) -> Result<(),CryptoError> do Crypto.mlkem_validate_public_parts(h,v) end";
#[test]
fn c1_public_validation_has_only_public_inputs_and_unit_result() {
    good(API);
    good("fn twice(h :: MlKemHeader64,v :: MlKemPublicVector1152) -> Result<(),CryptoError> do Crypto.mlkem_validate_public_parts(h,v)?\ncrypto_mlkem_validate_public_parts(h,v) end");
    good("fn stored(h :: MlKemHeader64,v :: MlKemPublicVector1152) do let op=Crypto.mlkem_validate_public_parts\nop(h,v) end");
    good("fn piped(h :: MlKemHeader64,v :: MlKemPublicVector1152) do h |> Crypto.mlkem_validate_public_parts(v) end");
    for source in [
        "fn bad(h :: Bytes,v :: MlKemPublicVector1152) do Crypto.mlkem_validate_public_parts(h,v) end",
        "fn bad(h :: borrow MlKemPrivateKey,v :: MlKemPublicVector1152) do Crypto.mlkem_validate_public_parts(h,v) end",
        "fn bad(h :: MlKemHeader64,v :: MlKemPublicVector1152) -> SecretBytes!CryptoError do Crypto.mlkem_validate_public_parts(h,v) end",
    ] {
        assert!(!check(source).errors.is_empty(), "accepted wrong type: {source}");
    }
}
#[test]
fn c1_public_validation_does_not_release_private_input_or_caller_control() {
    good(API);
    good("fn internal(h :: MlKemHeader64,private :: Plaintext<Bytes>) do Crypto.mlkem_validate_public_parts(h,MlKemPublicVector1152 {bytes:private}) end");
    for prefix in ["Crypto.", "crypto_"] {
        for source in [
            format!("fn leak(h :: MlKemHeader64,private :: Plaintext<Bytes>) do let result={prefix}mlkem_validate_public_parts(h,MlKemPublicVector1152 {{bytes:private}})\nprintln(String.from(Result.is_ok(result))) end"),
            format!("fn leak(v :: MlKemPublicVector1152,private :: Plaintext<Bytes>) do let result={prefix}mlkem_validate_public_parts(MlKemHeader64 {{bytes:private}},v)\nprintln(String.from(Result.is_ok(result))) end"),
            format!("fn leak(h :: MlKemHeader64,v :: MlKemPublicVector1152,private :: Plaintext<Bool>) do if private do let result={prefix}mlkem_validate_public_parts(h,v)\nprintln(String.from(Result.is_ok(result))) else nil end end"),
        ] {
            let result = check(&source);
            assert!(!result.errors.is_empty(), "private status escaped: {source}");
            assert!(result.errors.iter().all(|e| matches!(e,TypeError::PlaintextViolation {..})), "not a flow-only refusal: {source}\n{:?}",result.errors);
        }
    }
}
#[test]
fn c1_public_validation_lookalikes_keep_source_types() {
    good(API);
    good("fn crypto_mlkem_validate_public_parts(a :: Int,b :: Int) -> Int do a+b end\nfn use() -> Int do crypto_mlkem_validate_public_parts(3,4) end");
    let dependency = mesh_parser::parse(
        "pub fn mlkem_validate_public_parts(a :: Int,b :: Int) -> Int do a+b end",
    );
    let checked = mesh_typeck::check(&dependency);
    assert!(checked.errors.is_empty());
    let exports = mesh_typeck::collect_exports(&dependency, &checked);
    let mut imports = mesh_typeck::ImportContext::empty();
    imports.module_exports.insert(
        "Foreign.Crypto".into(),
        mesh_typeck::ModuleExports::new("Foreign.Crypto".into(), &exports),
    );
    let parsed = mesh_parser::parse(
        "import Foreign.Crypto\nfn use() -> Int do Crypto.mlkem_validate_public_parts(3,4) end",
    );
    let checked = mesh_typeck::check_with_imports(&parsed, &imports);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
}
