//! Type checking tests for affine resource ownership.

use mesh_typeck::error::TypeError;
use mesh_typeck::ty::{Ty, TyCon};
use mesh_typeck::{ImportContext, ModuleExports, TypeckResult};

fn check_source(source: &str) -> TypeckResult {
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "parse errors: {:?}", parse.errors());
    mesh_typeck::check(&parse)
}

fn resource_violations(result: &TypeckResult) -> Vec<&str> {
    result
        .errors
        .iter()
        .filter_map(|error| match error {
            TypeError::ResourceViolation { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect()
}

fn check_with_module(module_name: &str, module_source: &str, source: &str) -> TypeckResult {
    let module_parse = mesh_parser::parse(module_source);
    assert!(
        module_parse.ok(),
        "module parse errors: {:?}",
        module_parse.errors()
    );
    let module_typeck = mesh_typeck::check(&module_parse);
    assert!(
        module_typeck.errors.is_empty(),
        "module type errors: {:?}",
        module_typeck.errors
    );
    let exports = mesh_typeck::collect_exports(&module_parse, &module_typeck);
    let mut module = ModuleExports {
        module_name: module_name.to_string(),
        ..ModuleExports::default()
    };
    module.functions = exports.functions;
    module.struct_defs = exports.struct_defs;
    module.sum_type_defs = exports.sum_type_defs;
    module.service_defs = exports.service_defs;
    module.actor_defs = exports.actor_defs;
    module.private_names = exports.private_names;
    module.type_aliases = exports.type_aliases;
    module.resource_types = exports.resource_types;
    module.function_ownership = exports.function_ownership;

    let mut imports = ImportContext::empty();
    imports
        .module_exports
        .insert(module_name.to_string(), module);
    let parse = mesh_parser::parse(source);
    assert!(parse.ok(), "parse errors: {:?}", parse.errors());
    mesh_typeck::check_with_imports(&parse, &imports)
}

#[test]
fn rejects_use_after_direct_move() {
    let result = check_source(
        "resource SecretHandle\nfn misuse(handle :: SecretHandle) do\n  let moved = handle\n  handle\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `handle` was used after it moved"]
    );
}

#[test]
fn consume_parameter_moves_the_argument() {
    let result = check_source(
        "resource SecretHandle\nfn destroy(handle :: consume SecretHandle) do\n  nil\nend\nfn misuse(handle :: SecretHandle) do\n  destroy(handle)\n  handle\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `handle` was used after it moved"]
    );
}

#[test]
fn repeated_direct_borrows_do_not_move_the_argument() {
    let result = check_source(
        "resource SecretHandle\nfn inspect(handle :: borrow SecretHandle) do\n  nil\nend\nfn valid(handle :: SecretHandle) do\n  inspect(handle)\n  inspect(handle)\n  handle\nend",
    );

    assert!(
        result.errors.is_empty(),
        "expected repeated borrows to remain valid: {:?}",
        result.errors
    );
}

#[test]
fn secret_destroy_is_a_compiler_known_consume_call() {
    let result = check_source(
        "fn misuse(secret :: SecretBytes) do\n  Secret.destroy(secret)\n  Secret.destroy(secret)\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn ordinary_structs_containing_resources_are_affine() {
    let result = check_source(
        "struct Vault do\n  secret :: SecretBytes\nend\nfn misuse(secret :: SecretBytes) do\n  let vault = Vault { secret: secret }\n  let moved = vault\n  vault\nend",
    );

    assert!(result.type_registry.is_resource_name("Vault"));
    assert_eq!(
        resource_violations(&result),
        ["resource `vault` was used after it moved"]
    );
}

#[test]
fn constructing_a_resource_container_moves_resource_fields() {
    let result = check_source(
        "struct Vault do\n  secret :: SecretBytes\nend\nfn misuse(secret :: SecretBytes) do\n  let vault = Vault { secret: secret }\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn sum_variants_containing_resources_are_affine() {
    let result = check_source(
        "type SecretState do\n  Empty\n  Present(secret :: SecretBytes)\nend\nfn misuse(state :: SecretState) do\n  let moved = state\n  state\nend",
    );

    assert!(result.type_registry.is_resource_name("SecretState"));
    assert_eq!(
        resource_violations(&result),
        ["resource `state` was used after it moved"]
    );
}

#[test]
fn constructing_a_resource_sum_moves_its_payload() {
    let result = check_source(
        "type SecretState do\n  Empty\n  Present(secret :: SecretBytes)\nend\nfn misuse(secret :: SecretBytes) do\n  let state = Present(secret)\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn resource_sums_reject_mailbox_and_formatting_boundaries() {
    let result = check_source(
        "type SecretState do\n  Empty\n  Present(secret :: SecretBytes)\nend\nfn send_state(pid :: Pid<SecretState>, state :: SecretState) do\n  send(pid, state)\nend\nfn print_state(state :: SecretState) do\n  println(state)\nend",
    );

    assert_eq!(
        resource_violations(&result),
        [
            "resource `state` cannot cross an actor mailbox boundary",
            "resource `state` cannot be interpolated or formatted",
        ]
    );
    let state = Ty::Con(TyCon::new("SecretState"));
    for trait_name in ["Debug", "Eq", "Ord", "Hash", "Display"] {
        assert!(
            !result.trait_registry.has_impl(trait_name, &state),
            "resource sum unexpectedly implements {trait_name}"
        );
    }
}

#[test]
fn branch_merge_is_moved_if_either_branch_consumes() {
    let result = check_source(
        "fn maybe_destroy(secret :: SecretBytes, should_destroy :: Bool) do\n  if should_destroy do\n    nil\n  else\n    Secret.destroy(secret)\n  end\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn secret_random_result_pattern_establishes_an_owned_secret() {
    let result = check_source(
        "fn misuse() do\n  case Secret.random(32) do\n    Ok(secret) -> (Secret.destroy(secret), Secret.destroy(secret))\n    Err(_) -> (nil, nil)\n  end\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn loop_rejects_a_move_that_could_repeat() {
    let result = check_source(
        "fn misuse(secret :: SecretBytes, keep_running :: Bool) do\n  while keep_running do\n    Secret.destroy(secret)\n  end\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` may be moved more than once by this loop"]
    );
}

#[test]
fn borrowed_parameters_cannot_be_consumed_by_the_callee() {
    let result =
        check_source("fn invalid(secret :: borrow SecretBytes) do\n  Secret.destroy(secret)\nend");

    assert_eq!(
        resource_violations(&result),
        ["borrowed resource `secret` cannot be moved"]
    );
}

#[test]
fn unknown_call_modes_fail_closed_to_move() {
    let result = check_source(
        "fn destroy(secret :: SecretBytes) do\n  nil\nend\nfn misuse(secret :: SecretBytes) do\n  let indirect = destroy\n  indirect(secret)\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot be passed through generic or indirect call `indirect`"]
    );
}

#[test]
fn resource_containers_do_not_receive_implicit_value_traits() {
    let result = check_source("struct Vault do\n  secret :: SecretBytes\nend");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );

    let vault = Ty::Con(TyCon::new("Vault"));
    for trait_name in ["Debug", "Eq", "Ord", "Hash"] {
        assert!(
            !result.trait_registry.has_impl(trait_name, &vault),
            "resource container unexpectedly implements {trait_name}"
        );
    }
}

#[test]
fn extracting_a_resource_field_moves_the_whole_parent() {
    let result = check_source(
        "struct KeyPair do\n  public_key :: Bytes\n  private_key :: SecretBytes\nend\nfn split(pair :: KeyPair) do\n  let public = pair.public_key\n  let private = pair.private_key\n  pair.public_key\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `pair` was used after it moved"]
    );
}

#[test]
fn ownership_is_checked_inside_modules() {
    let result = check_source(
        "module Secrets do\n  fn misuse(secret :: SecretBytes) do\n    let moved = secret\n    secret\n  end\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn forward_transitive_resource_containment_suppresses_value_traits() {
    let result = check_source(
        "struct Outer do\n  inner :: Inner\nend\nstruct Inner do\n  secret :: SecretBytes\nend",
    );
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    assert!(result.type_registry.is_resource_name("Outer"));
    assert!(result.type_registry.is_resource_name("Inner"));

    for type_name in ["Outer", "Inner"] {
        let ty = Ty::Con(TyCon::new(type_name));
        for trait_name in ["Debug", "Eq", "Ord", "Hash"] {
            assert!(
                !result.trait_registry.has_impl(trait_name, &ty),
                "{type_name} unexpectedly implements {trait_name}"
            );
        }
    }
}

#[test]
fn explicit_derives_are_rejected_for_affine_types_without_registering_impls() {
    let result = check_source(
        "struct Vault do\n  secret :: SecretBytes\nend deriving(Debug, Eq, Ord, Hash, Display, Json, Row, Schema)",
    );

    assert_eq!(
        resource_violations(&result),
        [
            "resource type `Vault` cannot derive `Debug`",
            "resource type `Vault` cannot derive `Eq`",
            "resource type `Vault` cannot derive `Ord`",
            "resource type `Vault` cannot derive `Hash`",
            "resource type `Vault` cannot derive `Display`",
            "resource type `Vault` cannot derive `Json`",
            "resource type `Vault` cannot derive `Row`",
            "resource type `Vault` cannot derive `Schema`",
        ]
    );

    let vault = Ty::Con(TyCon::new("Vault"));
    for trait_name in [
        "Debug", "Eq", "Ord", "Hash", "Display", "ToJson", "FromJson", "FromRow", "Schema",
    ] {
        assert!(
            !result.trait_registry.has_impl(trait_name, &vault),
            "resource container unexpectedly implements {trait_name}"
        );
    }
}

#[test]
fn rejects_resource_string_interpolation() {
    let result = check_source("fn misuse(secret :: SecretBytes) do\n  println(\"#{secret}\")\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot be interpolated or formatted"]
    );
}

#[test]
fn rejects_resource_json_serialization() {
    let result = check_source("fn misuse(secret :: SecretBytes) do\n  Json.encode(secret)\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot cross JSON or serialization boundaries"]
    );
}

#[test]
fn rejects_resources_in_json_literals() {
    let result =
        check_source("fn misuse(secret :: SecretBytes) do\n  json { secret: secret }\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot cross JSON or serialization boundaries"]
    );
}

#[test]
fn rejects_resource_formatting_calls() {
    let result = check_source("fn misuse(secret :: SecretBytes) do\n  println(secret)\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot be interpolated or formatted"]
    );
}

#[test]
fn rejects_resource_equality() {
    let result =
        check_source("fn misuse(secret :: SecretBytes) -> Bool do\n  secret == secret\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot be compared or hashed"]
    );
}

#[test]
fn opaque_resource_names_are_not_value_constructors() {
    let result = check_source("resource SecretHandle\nlet forged = SecretHandle");

    assert!(
        result.errors.iter().any(
            |error| matches!(error, TypeError::UnboundVariable { name, .. } if name == "SecretHandle")
        ),
        "opaque resource was forgeable from its type name: {:?}",
        result.errors
    );
}

#[test]
fn module_nested_structs_receive_transitive_resource_metadata() {
    let result = check_source(
        "module Secure do\n  struct Outer do\n    inner :: Inner\n  end\n  struct Inner do\n    secret :: SecretBytes\n  end\nend",
    );

    assert!(result.type_registry.is_resource_name("Outer"));
    assert!(result.type_registry.is_resource_name("Inner"));
}

#[test]
fn rejects_typed_and_untyped_actor_resource_messages() {
    let result = check_source(
        "fn typed(pid :: Pid<SecretBytes>, secret :: SecretBytes) do\n  send(pid, secret)\nend\nfn untyped(pid :: Pid, secret :: SecretBytes) do\n  send(pid, secret)\nend",
    );

    assert_eq!(
        resource_violations(&result),
        [
            "resource `secret` cannot cross an actor mailbox boundary",
            "resource `secret` cannot cross an actor mailbox boundary",
        ]
    );
}

#[test]
fn rejects_resource_spawn_arguments() {
    let result = check_source(
        "actor holder(secret :: SecretBytes) do\n  receive do\n    _ -> holder(secret)\n  end\nend\nfn misuse(secret :: SecretBytes) do\n  spawn(holder, secret)\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot be transferred into a spawned actor"]
    );
}

#[test]
fn rejects_resources_in_unrestricted_lists() {
    let result = check_source(
        "fn misuse(secret :: SecretBytes) do\n  let values = [secret]\n  List.length(values)\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot enter an unrestricted collection"]
    );
}

#[test]
fn rejects_resources_in_unrestricted_maps() {
    let result = check_source(
        "fn misuse(secret :: SecretBytes) do\n  let values = %{\"secret\" => secret}\n  values\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot enter an unrestricted collection"]
    );
}

#[test]
fn rejects_resources_passed_to_collection_apis() {
    let result =
        check_source("fn misuse(secret :: SecretBytes) do\n  Set.add(Set.new(), secret)\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot enter an unrestricted collection"]
    );
}

#[test]
fn rejects_resource_closure_capture() {
    let result = check_source(
        "fn make_closure(secret :: SecretBytes) do\n  fn () -> Secret.destroy(secret) end\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot be captured by a closure"]
    );
}

#[test]
fn rejects_resource_bearing_collection_parameter_types() {
    let result = check_source("fn invalid(secrets :: List<SecretBytes>) do\n  secrets\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource-bearing type `List<SecretBytes>` cannot be used as an unrestricted collection"]
    );
}

#[test]
fn rejects_resource_bearing_collection_return_types() {
    let result = check_source("fn invalid() -> List<SecretBytes> do\n  []\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource-bearing type `List<SecretBytes>` cannot be used as an unrestricted collection"]
    );
}

#[test]
fn rejects_resource_bearing_collection_let_types() {
    let result =
        check_source("fn invalid() do\n  let secrets :: List<SecretBytes> = []\n  nil\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource-bearing type `List<SecretBytes>` cannot be used as an unrestricted collection"]
    );
}

#[test]
fn resource_tuple_construction_moves_resource_elements() {
    let result = check_source(
        "fn misuse(secret :: SecretBytes) do\n  let pair = (secret, 1)\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn rejects_resource_arguments_to_generic_borrow_parameters() {
    let result = check_source(
        "fn leak<T>(value :: borrow T) do\n  println(value)\nend\nfn misuse(secret :: SecretBytes) do\n  leak(secret)\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot be passed through generic or indirect call `leak`"]
    );
}

#[test]
fn rejects_resource_hash_method_calls() {
    let result = check_source("fn misuse(secret :: SecretBytes) do\n  secret.hash()\nend");

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` cannot be compared or hashed"]
    );
}

#[test]
fn for_loop_rejects_a_move_that_could_repeat() {
    let result = check_source(
        "fn misuse(secret :: SecretBytes) do\n  for _ in [1, 2] do\n    Secret.destroy(secret)\n  end\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` may be moved more than once by this loop"]
    );
}

#[test]
fn unmodified_resource_parameters_move_by_default_at_calls() {
    let result = check_source(
        "fn take(secret :: SecretBytes) do\n  nil\nend\nfn misuse(secret :: SecretBytes) do\n  take(secret)\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn option_resources_are_affine() {
    let result = check_source(
        "fn misuse(secret :: Option<SecretBytes>) do\n  let moved = secret\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn allows_resource_results_with_crypto_errors() {
    let result = check_source(
        "resource struct KeyPair do\n  private_key :: SecretBytes\nend\nfn accept(result :: Result<KeyPair, CryptoError>) do\n  nil\nend",
    );

    assert!(
        resource_violations(&result).is_empty(),
        "resource Result wrapper should be accepted: {:?}",
        result.errors
    );
}

#[test]
fn actor_bodies_track_resource_parameter_moves() {
    let result = check_source(
        "actor unsafe_holder(secret :: SecretBytes) do\n  Secret.destroy(secret)\n  Secret.destroy(secret)\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn actor_receive_branches_track_resource_parameter_moves() {
    let result = check_source(
        "actor unsafe_holder(secret :: SecretBytes) do\n  receive do\n    _ -> (Secret.destroy(secret), Secret.destroy(secret))\n  end\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn rejects_resource_valued_top_level_bindings() {
    let result = check_source("let secret = Secret.random(32)?");

    assert_eq!(
        resource_violations(&result),
        ["resource-bearing top-level binding `secret` is unsupported"]
    );
}

#[test]
fn duplicate_module_function_names_keep_qualified_ownership_modes() {
    let result = check_source(
        "module A do\n  fn take(secret :: borrow SecretBytes) do\n    nil\n  end\nend\nmodule B do\n  fn take(secret :: consume SecretBytes) do\n    nil\n  end\nend\nfn misuse(secret :: SecretBytes) do\n  A.take(secret)\n  B.take(secret)\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn imported_resource_types_remain_affine() {
    let result = check_with_module(
        "Secrets",
        "pub resource ForeignSecret",
        "from Secrets import ForeignSecret\nfn misuse(secret :: ForeignSecret) do\n  let moved = secret\n  secret\nend",
    );

    assert_eq!(
        resource_violations(&result),
        ["resource `secret` was used after it moved"]
    );
}

#[test]
fn qualified_imports_preserve_borrow_parameter_modes() {
    let result = check_with_module(
        "Secrets",
        "pub fn inspect(secret :: borrow SecretBytes) do\n  nil\nend",
        "import Secrets\nfn valid(secret :: SecretBytes) do\n  Secrets.inspect(secret)\n  Secrets.inspect(secret)\n  Secret.destroy(secret)\nend",
    );

    assert!(
        result.errors.is_empty(),
        "imported borrows should not move resources: {:?}",
        result.errors
    );
}
