//! Result outcomes must not silently pass the test harness. Runtime ABI unchanged.
use std::path::PathBuf;
use std::process::{Command, Output};
#[path = "support/test_artifacts.rs"]
mod artifacts;
fn compiler() -> PathBuf {
    std::env::var_os("MESH_TEST_OUTCOME_MESHC")
        .map(PathBuf::from)
        .unwrap_or_else(artifacts::meshc_bin)
}
fn run(source: &str, opt: u8) -> Output {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("mesh.toml"),
        "[package]\nname = \"test-outcomes\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(temp.path().join("main.mpl"), "fn main() do end\n").unwrap();
    let test = temp.path().join("outcome.test.mpl");
    std::fs::write(&test, source).unwrap();
    Command::new(compiler())
        .args([
            "test",
            test.to_str().unwrap(),
            "--opt-level",
            &opt.to_string(),
        ])
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}
fn streams(o: &Output) -> (String, String) {
    (
        String::from_utf8(o.stdout.clone()).unwrap(),
        String::from_utf8(o.stderr.clone()).unwrap(),
    )
}
#[test]
fn test_returned_err_and_propagation_fail_without_formatting_payload() {
    for opt in [0, 2] {
        let out = run(
            r#"fn fail() -> Result<(), String> do Err("DO_NOT_FORMAT_PRIVATE_ERROR") end
 test("tail Err") do fail() end
 test("propagated Err") do fail()?; println("UNREACHABLE_PROPAGATED"); Ok(nil) end
 test("explicit Err") do return Err("DO_NOT_FORMAT_PRIVATE_ERROR"); Ok(nil) end
 test("Ok") do println("OK_REACHED"); Ok(nil) end
 test("Unit") do println("UNIT_REACHED"); assert(true) end
 test("panic") do panic("EXPECTED_PANIC") end
 test("after") do println("AFTER_REACHED") end
"#,
            opt,
        );
        let (stdout, stderr) = streams(&out);
        assert!(!out.status.success(), "opt{opt}: {stdout}{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
        for expected in [
            "4 failed",
            "3 passed",
            "OK_REACHED",
            "UNIT_REACHED",
            "AFTER_REACHED",
            "EXPECTED_PANIC",
        ] {
            assert!(stdout.contains(expected), "missing {expected}: {stdout}");
        }
        assert_eq!(
            stdout.matches("test step returned Err").count(),
            6,
            "{stdout}"
        );
        for secret in ["DO_NOT_FORMAT_PRIVATE_ERROR", "UNREACHABLE_PROPAGATED"] {
            assert!(!stdout.contains(secret), "{stdout}");
        }
    }
}
#[test]
fn test_result_setup_body_and_teardown_preserve_step_order() {
    for opt in [0, 2] {
        let out = run(
            r#"fn fail() -> Result<(), String> do Err("DO_NOT_FORMAT_PRIVATE_ERROR") end
 describe("bad setup value") do
   setup do fail() end
   teardown do println("UNREACHABLE_BAD_SETUP_TEARDOWN") end
   test("body") do println("UNREACHABLE_BAD_SETUP_BODY") end
 end
 describe("bad setup propagation") do
   setup do fail()?; println("UNREACHABLE_SETUP_PROPAGATION") end
   test("body") do println("UNREACHABLE_PROPAGATED_SETUP_BODY") end
 end
 describe("failed body") do
   setup do let captured="SCOPED" end
   teardown do println("BODY_TEARDOWN #{captured}"); Ok(nil) end
   test("Err") do fail() end
 end
 describe("failed teardown") do
   teardown do fail() end
   test("body") do println("TEARDOWN_BODY_REACHED") end
 end
 describe("both") do
   teardown do fail() end
   test("body") do fail() end
 end
 describe("nested setup") do
   teardown do println("UNREACHABLE_OUTER_TEARDOWN") end
   describe("inner") do
     setup do fail() end
     teardown do println("UNREACHABLE_INNER_TEARDOWN") end
     test("not entered") do println("UNREACHABLE_INNER_BODY") end
   end
 end
 test("after") do println("LATER_REACHED") end
"#,
            opt,
        );
        let (stdout, stderr) = streams(&out);
        assert!(!out.status.success(), "opt{opt}: {stdout}{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
        for expected in [
            "6 failed",
            "1 passed",
            "BODY_TEARDOWN SCOPED",
            "TEARDOWN_BODY_REACHED",
            "LATER_REACHED",
        ] {
            assert!(stdout.contains(expected), "missing {expected}: {stdout}");
        }
        assert_eq!(
            stdout.matches("test step returned Err").count(),
            14,
            "{stdout}"
        );
        assert!(!stdout.contains("UNREACHABLE"), "{stdout}");
        assert!(!stdout.contains("DO_NOT_FORMAT_PRIVATE_ERROR"), "{stdout}");
    }
}
#[test]
fn test_returned_owned_ok_and_err_payloads_are_destroyed_before_failure() {
    for opt in [0, 2] {
        let out = run(
            r#"fn owned_error() -> Result<(), SecretBytes> do
   case Secret.random(65536) do Err(_) -> panic("OWNED_ALLOC_FAILED") Ok(value) -> Err(value) end
 end
 describe("owned Ok setup") do
   setup do Secret.random(65536) end
   test("empty") do nil end
 end
 describe("owned Err body") do
   test("fails") do owned_error() end
 end
 test("repeat actual generated steps within one actor") do
   # White-box runner regression: exceeding the 4MiB actor budget would fail if
   # even one of these discarded 64KiB Ok or Err payloads remained owned.
   for i in 0..100 do
     __test_describe_0(0)
     __test_describe_1(0)
   end
   println("OWNED_CLEANUP_LOOP_COMPLETED")
 end
 test("later") do println("OWNED_AFTER_REACHED") end
"#,
            opt,
        );
        let (stdout, stderr) = streams(&out);
        assert!(!out.status.success(), "opt{opt}: {stdout}{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
        assert!(stdout.contains("OWNED_CLEANUP_LOOP_COMPLETED"), "{stdout}");
        assert!(stdout.contains("OWNED_AFTER_REACHED"), "{stdout}");
        assert!(!stdout.contains("OWNED_ALLOC_FAILED"), "{stdout}");
        assert!(
            stdout.contains("2 failed") && stdout.contains("2 passed"),
            "{stdout}"
        );
        assert_eq!(
            stdout.matches("test step returned Err").count(),
            202,
            "{stdout}"
        );
    }
}
#[test]
fn test_outcome_observer_is_private_and_normal_closure_results_stay_values() {
    let out = run(
        "fn __test_check_result(value) do nil end\ntest(\"collision\") do assert(true) end\n",
        0,
    );
    let (stdout, stderr) = streams(&out);
    assert!(!out.status.success(), "{stdout}{stderr}");
    assert!(
        stdout.contains("reserved for the generated test harness"),
        "{stdout}{stderr}"
    );
    let out = run(
        r#"fn ordinary() -> Result<(),String> do Err("ordinary") end
 test("ordinary results remain values") do
   let closure=fn() do ordinary() end
   assert(Result.is_err(closure()))
   assert(Result.is_err(ordinary()))
 end
"#,
        0,
    );
    let (stdout, stderr) = streams(&out);
    assert!(out.status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("1 passed"), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn test_nested_setup_receive_rewrite_runs_and_refuses_wrong_message() {
    for opt in [0, 2] {
        for message in [40, 7] {
            let source = format!(
                r#"describe("nested setup receive") do
  setup do
    send(self(), {message})
    if true do
      assert_receive 40, 500
    end
  end
  teardown do println("RECEIVE_TEARDOWN_REACHED") end
  test("body") do println("RECEIVE_BODY_REACHED") end
end
test("later") do println("RECEIVE_LATER_REACHED") end
"#
            );
            let out = run(&source, opt);
            let (stdout, stderr) = streams(&out);
            assert!(stderr.is_empty(), "{stderr}");
            assert!(stdout.contains("RECEIVE_LATER_REACHED"), "{stdout}");
            if message == 40 {
                assert!(out.status.success(), "{stdout}");
                assert!(
                    stdout.contains("RECEIVE_BODY_REACHED")
                        && stdout.contains("RECEIVE_TEARDOWN_REACHED")
                        && stdout.contains("2 passed"),
                    "{stdout}"
                );
            } else {
                assert!(!out.status.success(), "{stdout}");
                assert!(
                    stdout.contains("assert_receive 40 received another message")
                        && stdout.contains("1 failed")
                        && stdout.contains("1 passed"),
                    "{stdout}"
                );
                assert!(
                    !stdout.contains("RECEIVE_BODY_REACHED")
                        && !stdout.contains("RECEIVE_TEARDOWN_REACHED"),
                    "{stdout}"
                );
            }
        }
    }
}
