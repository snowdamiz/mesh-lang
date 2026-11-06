use std::process::Command;

#[path = "support/test_artifacts.rs"]
mod artifacts;

#[test]
fn typed_process_lookup_boxes_a_real_pid_and_observes_exit_in_debug_and_optimized_code() {
    for optimized in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("process-lookup");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(
            project.join("main.mpl"),
            r#"
actor wait() do
  receive do
    _message -> nil
  end
end
fn main() do
  case Process.lookup("process-lookup-missing") do
    None -> println("missing")
    Some(_) -> println("bad")
  end
  let child :: Pid<Int> = spawn(wait)
  if Process.register("process-lookup-child", child) != 0 do
    println("bad registration")
  end
  case Process.lookup("process-lookup-child") do
    Some(pid) -> do
      if pid == Process.whereis("process-lookup-child") do
        println("found")
      end
      send(pid, 1)
      nil
    end
    None -> println("bad absence")
  end
  Timer.sleep(150)
  case Process.lookup("process-lookup-child") do
    None -> println("exited")
    Some(_) -> println("bad stale")
  end
end
"#,
        )
        .unwrap();
        let mut command = Command::new(artifacts::meshc_bin());
        command.args(["build", project.to_str().unwrap()]);
        if optimized {
            command.args(["--opt-level", "2"]);
        }
        let build = command.output().unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let output = Command::new(project.join("process-lookup"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "missing\nfound\nexited\n"
        );
    }
}
