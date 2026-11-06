//! Child-isolated execution of both public OHTTP API names.
use super::*;

#[test]
fn ohttp_both_public_names_execute_all_four_opaque_context_operations() {
    if std::env::var_os("MESH_M3_OHTTP_JIT_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "jit::ohttp_tests::ohttp_both_public_names_execute_all_four_opaque_context_operations", "--nocapture"])
            .env("MESH_M3_OHTTP_JIT_CHILD", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout
                .lines()
                .any(|line| line == "mesh:m3:ohttp:jit:both-names-four-symbols-validated"),
            "child did not finish the actual API assertions: {stdout}"
        );
        assert!(
            stdout.contains("1 passed; 0 failed"),
            "exact child selected zero or failed cases: {stdout}"
        );
        return;
    }
    init_runtime();
    for prefix in ["Ohttp.", "ohttp_"] {
        let mut session = ReplSession::new();
        let source = format!(
            r#"
fn ohttp_probe() -> Int!CryptoError do
  let pair = Crypto.x25519_generate()?
  let body = Bytes.from_utf8("jit request")
  let reply_body = Bytes.from_utf8("jit response")
  let packet = {prefix}encapsulate_request(37, pair.public_key, body)?
  let wire = packet.wire
  let opened = {prefix}decapsulate_request(pair.private_key, 37, wire)?
  let clear_matches = Bytes.secure_equals(opened.plaintext, body)
  let response = {prefix}encapsulate_response(opened.response_context, reply_body)?
  let clear_response = {prefix}decapsulate_response(packet.response_context, response)?
  Secret.destroy(pair.private_key)
  Ok(if clear_matches && Bytes.secure_equals(clear_response, reply_body) && Bytes.length(wire) == Bytes.length(body) + 55 do 71 else 0 end)
end
"#
        );
        jit_eval(&source, &mut session).unwrap_or_else(|e| panic!("{source}: {e}"));
        let result = jit_eval("Result.unwrap_or(ohttp_probe(), 0)", &mut session).unwrap();
        assert_eq!(result.value, "71", "{prefix}");
        assert_eq!(result.ty, "Int");
    }
    let symbols = runtime_symbols();
    for name in [
        "mesh_ohttp_encapsulate_request",
        "mesh_ohttp_decapsulate_request",
        "mesh_ohttp_encapsulate_response",
        "mesh_ohttp_decapsulate_response",
    ] {
        assert!(
            symbols
                .iter()
                .any(|(registered, pointer)| *registered == name && !pointer.is_null()),
            "missing {name}"
        );
    }
    // Keep the independent existing C4 JIT marker/assertions unchanged.
    println!("\nmesh:m3:ohttp:jit:both-names-four-symbols-validated");
}
