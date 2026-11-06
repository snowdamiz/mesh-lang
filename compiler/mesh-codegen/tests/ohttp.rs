//! OHTTP MIR ownership and LLVM ABI contracts.
use inkwell::context::Context;
use mesh_codegen::codegen::intrinsics::declare_intrinsics;
use mesh_codegen::mir::{MirExpr, MirModule, MirResourceDestructor, MirType};
use std::collections::{HashMap, HashSet};

fn lower(source: &str) -> MirModule {
    let parsed = mesh_parser::parse(source);
    assert!(parsed.ok(), "{:?}", parsed.errors());
    let checked = mesh_typeck::check(&parsed);
    assert!(checked.errors.is_empty(), "{source}\n{:?}", checked.errors);
    mesh_codegen::mir::lower::lower_to_mir(&parsed, &checked, "", &HashSet::new(), &HashMap::new())
        .unwrap()
}

#[test]
fn ohttp_mir_maps_both_names_and_preserves_borrow_versus_consume() {
    for prefix in ["Ohttp.", "ohttp_"] {
        let source = format!(
            r#"
fn request(id :: Int, key :: X25519PublicKey, body :: Bytes) -> OhttpRequest!CryptoError do {prefix}encapsulate_request(id, key, body) end
fn open(key :: borrow X25519PrivateKey, id :: Int, body :: Bytes) -> OhttpRequestOpening!CryptoError do {prefix}decapsulate_request(key, id, body) end
fn reply(context :: OhttpGatewayContext, body :: Bytes) -> Bytes!CryptoError do {prefix}encapsulate_response(context, body) end
fn receive_reply(context :: OhttpClientContext, body :: Bytes) -> Bytes!CryptoError do {prefix}decapsulate_response(context, body) end
"#
        );
        let module = lower(&source);
        for (function, runtime, arity) in [
            ("request", "mesh_ohttp_encapsulate_request", 3),
            ("open", "mesh_ohttp_decapsulate_request", 3),
            ("reply", "mesh_ohttp_encapsulate_response", 2),
            ("receive_reply", "mesh_ohttp_decapsulate_response", 2),
        ] {
            let body = &module
                .functions
                .iter()
                .find(|f| f.name == function)
                .unwrap()
                .body;
            let arguments = body.descendants().into_iter().find_map(|node| match node {
                MirExpr::Call { func, args, .. } if matches!(func.as_ref(), MirExpr::Var(name, _) if name == runtime) => Some(args),
                _ => None,
            }).unwrap_or_else(|| panic!("missing {runtime}: {body:?}"));
            assert_eq!(arguments.len(), arity);
            if function == "open" {
                assert!(
                    matches!(arguments[0], MirExpr::ResourceBorrow { .. }),
                    "{body:?}"
                );
            } else if function == "reply" || function == "receive_reply" {
                assert!(
                    matches!(arguments[0], MirExpr::ResourceMove { .. }),
                    "{body:?}"
                );
            }
        }
    }
}

#[test]
fn ohttp_records_have_two_pointer_layouts_and_drop_only_the_context_field() {
    for (record, first) in [
        ("OhttpRequest", "wire"),
        ("OhttpRequestOpening", "plaintext"),
    ] {
        let module = lower(&format!("fn discard(packet :: {record}) do nil end"));
        let layout = module.structs.iter().find(|s| s.name == record).unwrap();
        assert_eq!(
            layout.fields,
            vec![
                (first.to_string(), MirType::Ptr),
                ("response_context".to_string(), MirType::Ptr)
            ]
        );
        let body = &module
            .functions
            .iter()
            .find(|f| f.name == "discard")
            .unwrap()
            .body;
        let destructor = body
            .descendants()
            .into_iter()
            .find_map(|node| match node {
                MirExpr::ResourceDrop { destructor, .. } => Some(destructor),
                _ => None,
            })
            .unwrap();
        let MirResourceDestructor::Aggregate(fields) = destructor else {
            panic!("wrong destructor {destructor:?}");
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].index, 1);
        assert!(matches!(
            fields[0].destructor,
            MirResourceDestructor::Opaque
        ));
    }
}

#[test]
fn ohttp_llvm_intrinsics_match_the_public_pointer_and_integer_abi() {
    let context = Context::create();
    let module = context.create_module("ohttp-abi");
    declare_intrinsics(&module);
    for (name, integers) in [
        ("mesh_ohttp_encapsulate_request", vec![true, false, false]),
        ("mesh_ohttp_decapsulate_request", vec![false, true, false]),
        ("mesh_ohttp_encapsulate_response", vec![false, false]),
        ("mesh_ohttp_decapsulate_response", vec![false, false]),
    ] {
        let function = module
            .get_function(name)
            .unwrap_or_else(|| panic!("missing {name}"));
        let parameters = function.get_type().get_param_types();
        assert_eq!(parameters.len(), integers.len(), "{name}");
        for (parameter, integer) in parameters.iter().zip(integers) {
            if integer {
                assert!(parameter.is_int_type(), "{name}");
                assert_eq!(parameter.into_int_type().get_bit_width(), 64, "{name}");
            } else {
                assert!(parameter.is_pointer_type(), "{name}");
            }
        }
        assert!(
            function
                .get_type()
                .get_return_type()
                .unwrap()
                .is_pointer_type(),
            "{name}"
        );
    }
}
