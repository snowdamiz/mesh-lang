//! Hook inside jit.rs; retain C4 and C1 existing marker tests.
use super::*;
fn check_symbols() {
    let list = runtime_symbols();
    for name in [
        "mesh_crypto_hkdf_sha256_secret_salt",
        "mesh_secret_split_at",
        "mesh_crypto_hmac_sha256_tag",
    ] {
        assert!(
            list.iter().any(|(n, p)| *n == name && !p.is_null()),
            "missing genuine symbol {name}"
        );
    }
}
#[test]
fn c1_secret_primitives_jit_three_symbols_are_registered() {
    check_symbols();
}
#[test]
fn c1_secret_primitives_execute_in_one_marked_fresh_child() {
    const NAME: &str =
        "jit::secret_primitives_tests::c1_secret_primitives_execute_in_one_marked_fresh_child";
    const MARKER: &str = "C1_SECRET_PRIMITIVES_JIT_EXECUTED";
    if std::env::var("MESH_C1_SECRET_PRIMITIVES_JIT_CHILD").as_deref() != Ok("1") {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env("MESH_C1_SECRET_PRIMITIVES_JIT_CHILD", "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success()
                && stdout.contains(MARKER)
                && stdout.contains("1 passed; 0 failed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    check_symbols();
    init_runtime();
    let mut s = ReplSession::new();
    jit_eval(include_str!("secret_primitives_jit_probe.mpl"), &mut s).unwrap();
    let result = jit_eval("Result.unwrap_or(c1_secret_primitives_probe(),0)", &mut s).unwrap();
    assert_eq!(result.value, "73");
    assert_eq!(result.ty, "Int");
    println!("{MARKER}");
}
