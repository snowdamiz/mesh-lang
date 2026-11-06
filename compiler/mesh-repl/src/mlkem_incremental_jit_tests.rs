use super::*;

const SYMBOLS: &[&str] = &[
    "mesh_crypto_mlkem_validate_public_parts",
    "mesh_crypto_mlkem_public_parts",
    "mesh_crypto_mlkem_encapsulate1",
    "mesh_crypto_mlkem_encapsulate2",
    "mesh_crypto_mlkem_decapsulate_parts",
    "mesh_mlkem_encapsulation_state_seal_for_storage",
    "mesh_mlkem_encapsulation_state_unseal_from_storage",
];

fn checked_symbols() {
    let symbols = runtime_symbols();
    for name in SYMBOLS {
        assert!(
            symbols
                .iter()
                .any(|(registered, pointer)| registered == name && !pointer.is_null()),
            "incremental native symbol missing from JIT registry: {name}"
        );
    }
}

#[test]
fn c1_incremental_runtime_symbols_are_registered() {
    checked_symbols();
}

#[test]
fn c1_incremental_tuple_retry_and_storage_execute_in_a_fresh_jit_child() {
    const NAME: &str = "jit::incremental_tests::c1_incremental_tuple_retry_and_storage_execute_in_a_fresh_jit_child";
    const MARKER: &str = "C1_INCREMENTAL_JIT_EXECUTED";
    if std::env::var("MESH_C1_JIT_CHILD").as_deref() != Ok("1") {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env("MESH_C1_JIT_CHILD", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success()
                && stdout.contains(MARKER)
                && stdout.contains("1 passed; 0 failed"),
            "exact JIT child must execute one marked test: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    // Fail deterministically before any unresolved JIT call on the pre-map RED
    // source, instead of allowing an unresolved native function to jump to null.
    checked_symbols();
    init_runtime();
    let mut session = ReplSession::new();
    for definition in include_str!("mlkem_incremental_jit_probe.mpl").split("\n#--definition--\n") {
        jit_eval(definition, &mut session).unwrap_or_else(|error| panic!("{definition}: {error}"));
    }
    let result = jit_eval("Result.unwrap_or(c1_incremental_probe(), 0)", &mut session)
        .unwrap_or_else(|error| panic!("incremental JIT probe: {error}"));
    assert_eq!(result.value, "71");
    assert_eq!(result.ty, "Int");
    println!("{MARKER}");
}
