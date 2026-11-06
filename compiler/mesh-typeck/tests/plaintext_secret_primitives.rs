//! Synthetic public-library flow tests only; no app ingress policy or disclosures.
//! Generic labels must not disappear at a tag or opaque split.
use mesh_typeck::{error::TypeError, TypeckResult};
fn check(s: &str) -> TypeckResult {
    let p = mesh_parser::parse(s);
    assert!(p.ok(), "{:?}", p.errors());
    mesh_typeck::check(&p)
}
fn good(s: &str) {
    let r = check(s);
    assert!(r.errors.is_empty(), "{s}: {:?}", r.errors);
}
fn refused(s: &str) {
    let r = check(s);
    assert!(!r.errors.is_empty(), "accepted {s}");
    assert!(
        r.errors
            .iter()
            .all(|e| matches!(e, TypeError::PlaintextViolation { .. })),
        "not a flow-only refusal {s}: {:?}",
        r.errors
    );
}
const API:&str="fn tag(key :: borrow SecretBytes,message :: Bytes) -> Bytes!CryptoError do Crypto.hmac_sha256_tag(key,message) end\nfn split(value :: consume SecretBytes) do Secret.split_at(value,1) end\nfn derive(a :: borrow SecretBytes,b :: borrow SecretBytes,info :: Bytes) do Crypto.hkdf_sha256_secret_salt(a,b,info,32) end";
#[test]
fn secret_primitives_public_context_and_internal_private_derivation_are_allowed() {
    good(API);
    good("fn internal(a :: borrow SecretBytes,b :: borrow SecretBytes,info :: Plaintext<Bytes>) do Crypto.hkdf_sha256_secret_salt(a,b,info,32) end");
    good("fn public_tag(key :: borrow SecretBytes) -> Bytes!CryptoError do Crypto.hmac_sha256_tag(key,Bytes.from_utf8(\"public transcript\")) end");
}
#[test]
fn secret_primitives_full_tag_does_not_disclose_labeled_message_or_caller_control() {
    good(API);
    for s in [
 "fn leak(key :: borrow SecretBytes,value :: Plaintext<Bytes>) do let tag=Result.unwrap_or(Crypto.hmac_sha256_tag(key,value),Bytes.empty())\nprintln(Bytes.to_hex(tag)) end",
 "fn leak(key :: borrow SecretBytes,flag :: Plaintext<Bool>) do if flag do let tag=Result.unwrap_or(Crypto.hmac_sha256_tag(key,Bytes.empty()),Bytes.empty())\nprintln(Bytes.to_hex(tag)) else nil end end",
 "fn leak(key :: borrow SecretBytes,value :: Plaintext<Bytes>) do let tag=Result.unwrap_or(crypto_hmac_sha256_tag(key,value),Bytes.empty())\nHttp.body_bytes(Http.build(:post,\"https://example.test\"),tag) end",
 ]{refused(s);}
}
#[test]
fn secret_primitives_secret_salt_derived_and_split_outputs_keep_dependencies() {
    good(API);
    refused("fn leak(a :: borrow SecretBytes,b :: borrow SecretBytes,info :: Plaintext<Bytes>) -> Result<(),CryptoError> do let derived=Crypto.hkdf_sha256_secret_salt(a,b,info,64)?\nlet (left,right)=Secret.split_at(derived,32)?\nlet tag=Crypto.hmac_sha256_tag(left,Bytes.empty())?\nprintln(Bytes.to_hex(tag))\nOk(nil) end");
    refused("fn leak(value :: SecretBytes,index :: Plaintext<Int>) do case Secret.split_at(value,index) do Err(_) -> println(\"refused\") Ok((left,right)) -> do Secret.destroy(left)\nSecret.destroy(right)\nprintln(\"accepted\") end end end");
}
#[test]
fn secret_primitives_lookalike_names_do_not_acquire_disclosure_authority() {
    good(API);
    refused("module Crypto do pub fn hmac_sha256_tag(key :: borrow SecretBytes,value :: Bytes) -> Bytes!CryptoError do println(Bytes.to_hex(value))\nOk(value) end end\nfn leak(key :: borrow SecretBytes,value :: Plaintext<Bytes>) do Crypto.hmac_sha256_tag(key,value) end");
}
