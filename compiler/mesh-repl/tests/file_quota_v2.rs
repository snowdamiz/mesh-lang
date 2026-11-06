//! Real REPL file IO, isolated so a missing JIT symbol cannot kill the harness.
use mesh_repl::{jit::init_runtime, jit_eval, ReplSession};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[test]
#[ignore = "invoked only by the bounded actual REPL IO tests"]
fn file_quota_v2_actual_repl_child() {
    let operation = std::env::var("FILE_QUOTA_REPL_OPERATION").unwrap();
    let root = PathBuf::from(std::env::var("FILE_QUOTA_REPL_ROOT").unwrap());
    init_runtime();
    if operation == "recovery" {
        println!("REPL_RECOVERY_CODE_CALL_REACHED");
        std::io::stdout().flush().unwrap();
        let mut session = ReplSession::new();
        jit_eval(
            r#"fn recovery_code_repl_probe() -> Bool!CryptoError do
              let (key, code) = RecoveryCode.generate()?
              let imported = RecoveryCode.parse(code)?
              let expected = Crypto.hmac_sha256_tag(key, Bytes.from_utf8("public recovery agreement"))?
              let actual = Crypto.hmac_sha256_tag(imported, Bytes.from_utf8("public recovery agreement"))?
              Secret.destroy(key)
              Secret.destroy(imported)
              Ok(Bytes.secure_equals(actual, expected))
            end"#,
            &mut session,
        )
        .unwrap_or_else(|error| panic!("REPL_RECOVERY_CODE_PIPELINE_ERROR: {error}"));
        let result = jit_eval(
            "Result.unwrap_or(recovery_code_repl_probe(), false)",
            &mut session,
        )
        .unwrap_or_else(|error| panic!("REPL_RECOVERY_CODE_PIPELINE_ERROR: {error}"));
        assert_eq!(result.value, "true", "actual REPL recovery agreement");
        assert_eq!(result.ty, "Bool");
        println!("REPL_RECOVERY_CODE_ROUNDTRIP_OK");
        return;
    }
    if operation == "sqlite" {
        println!("REPL_SQLITE_V2_CALL_REACHED");
        std::io::stdout().flush().unwrap();
        let mut session = ReplSession::new();
        jit_eval(r#"fn sqlite_quota_probe() -> Bool!String do
          let db = Sqlite.open(":memory:")?
          let _ = Sqlite.execute(db,"CREATE TABLE archive(payload BLOB NOT NULL)",[])?
          let rows = Sqlite.query_values_v2(db,"SELECT zeroblob(16797311) AS payload",[],16797311)?
          let data = case Map.get(List.head(rows),"payload") do Binary(value) -> value _ -> return Err("not_binary") end
          let first = Sqlite.execute_values_v2(db,"INSERT INTO archive VALUES (?)",[Binary(data)],16797311)?
          let second = sqlite_execute_values_v2(db,"INSERT INTO archive VALUES (?)",[Binary(data)],16797311)?
          let selected = sqlite_query_values_v2(db,"SELECT payload FROM archive LIMIT 1",[],16797311)?
          let actual = case Map.get(List.head(selected),"payload") do Binary(value) -> value _ -> return Err("not_binary") end
          let refused = Result.is_err(Sqlite.query_values(db,"SELECT payload FROM archive LIMIT 1",[]))
          Sqlite.close(db)
          Ok(first == 1 && second == 1 && Bytes.length(actual) == 16797311 && Bytes.secure_equals(actual,data) && refused)
        end"#, &mut session).unwrap();
        let result =
            jit_eval("Result.unwrap_or(sqlite_quota_probe(),false)", &mut session).unwrap();
        assert_eq!(result.value, "true");
        assert_eq!(result.ty, "Bool");
        println!("REPL_SQLITE_V2_ROUNDTRIP_OK");
        return;
    }
    let names = match operation.as_str() {
        "read" => ["File.read_bytes_v2", "file_read_bytes_v2"],
        "write" => ["File.write_bytes_v2", "file_write_bytes_v2"],
        _ => panic!("invalid test-only operation"),
    };
    for (index, name) in names.into_iter().enumerate() {
        let path = root.join(format!("file-{index}"));
        let literal = format!("{:?}", path.to_str().unwrap());
        let source = if operation == "read" {
            let mut file = fs::File::create(&path).unwrap();
            file.set_len(16_796_864).unwrap();
            file.seek(SeekFrom::Start(16_796_863)).unwrap();
            file.write_all(b"x").unwrap();
            format!("case {name}({literal}, 16796863, 1, 16796864) do Ok(value) -> Bytes.secure_equals(value, Bytes.from_utf8(\"x\")) Err(_) -> false end")
        } else {
            format!("case {name}({literal}, 16796863, Bytes.from_utf8(\"x\"), false, 16796864) do Ok(_) -> true Err(_) -> false end")
        };
        println!("REPL_FILE_V2_{operation}_CALL_REACHED {name}");
        std::io::stdout().flush().unwrap();
        let result = jit_eval(&source, &mut ReplSession::new())
            .unwrap_or_else(|error| panic!("REPL_FILE_V2_PIPELINE_ERROR: {error}"));
        assert_eq!(result.value, "true", "actual REPL {name} result");
        assert_eq!(result.ty, "Bool");
        assert_eq!(fs::metadata(&path).unwrap().len(), 16_796_864);
        let mut file = fs::File::open(&path).unwrap();
        file.seek(SeekFrom::Start(16_796_863)).unwrap();
        let mut tail = [0; 1];
        file.read_exact(&mut tail).unwrap();
        assert_eq!(tail, *b"x");
        println!("REPL_FILE_V2_{operation}_SPELLING_{index}_OK");
    }
}

fn actual_repl_io_refuses_missing_symbols(operation: &str) {
    struct OwnedProbe(Child);
    impl Drop for OwnedProbe {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    struct OwnedRoot(PathBuf);
    impl Drop for OwnedRoot {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let root = OwnedRoot(std::env::temp_dir().join(format!(
        "mesh-file-quota-repl-{}-{operation}",
        std::process::id()
    )));
    fs::create_dir(&root.0).unwrap();
    let stdout_path = root.0.join("stdout");
    let stderr_path = root.0.join("stderr");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "file_quota_v2_actual_repl_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("FILE_QUOTA_REPL_OPERATION", operation)
        .env("FILE_QUOTA_REPL_ROOT", &root.0)
        .stdout(Stdio::from(fs::File::create(&stdout_path).unwrap()))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()))
        .spawn()
        .unwrap();
    let mut child = OwnedProbe(child);
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if status.is_none() {
        child.0.kill().unwrap();
        child.0.wait().unwrap();
    }
    let stdout = fs::read_to_string(stdout_path).unwrap();
    let stderr = fs::read_to_string(stderr_path).unwrap();
    println!("REPL_FILE_V2_{operation}_CHILD stdout={stdout:?}; stderr={stderr:?}");
    println!(
        "REPL_FILE_V2_{operation}_CHILD_STATUS={}",
        status
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "timeout".into())
    );
    assert!(
        status.is_some(),
        "actual REPL {operation} timed out; child killed and reaped"
    );
    let reached = if operation == "sqlite" {
        "REPL_SQLITE_V2_CALL_REACHED".to_owned()
    } else if operation == "recovery" {
        "REPL_RECOVERY_CODE_CALL_REACHED".to_owned()
    } else {
        format!("REPL_FILE_V2_{operation}_CALL_REACHED")
    };
    assert!(stdout.contains(&reached), "actual REPL call not reached");
    assert!(
        status.unwrap().success(),
        "actual REPL {operation} child failed"
    );
    if operation == "sqlite" {
        assert!(stdout.contains("REPL_SQLITE_V2_ROUNDTRIP_OK"));
    } else if operation == "recovery" {
        assert!(stdout.contains("REPL_RECOVERY_CODE_ROUNDTRIP_OK"));
    } else {
        for index in 0..2 {
            assert!(stdout.contains(&format!("REPL_FILE_V2_{operation}_SPELLING_{index}_OK")));
        }
    }
}

#[test]
fn file_quota_v2_read_executes_namespace_and_flat_repl_calls() {
    actual_repl_io_refuses_missing_symbols("read");
}

#[test]
fn file_quota_v2_write_executes_namespace_and_flat_repl_calls() {
    actual_repl_io_refuses_missing_symbols("write");
}

#[test]
fn recovery_code_generate_parse_agree_in_repl() {
    actual_repl_io_refuses_missing_symbols("recovery");
}

#[test]
fn sqlite_explicit_quota_executes_namespace_and_flat_repl_calls() {
    actual_repl_io_refuses_missing_symbols("sqlite");
}
