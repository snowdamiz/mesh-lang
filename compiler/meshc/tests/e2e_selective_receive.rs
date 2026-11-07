//! Compiled selective receive, including heap payloads and effectful guards.
use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "support/test_artifacts.rs"]
mod artifacts;

fn run(source: &str, optimized: bool) -> String {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("selective");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("main.mpl"), source).unwrap();
    let output = Command::new(artifacts::meshc_bin())
        .arg("build")
        .arg(&project)
        .args(["--opt-level", if optimized { "2" } else { "0" }])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    let child = Command::new(project.join("selective"))
        .env("MESH_GC_STRESS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = artifacts::wait_with_timeout(child, Duration::from_secs(30)).unwrap();
    assert!(
        output.status.success(),
        "{}",
        artifacts::command_output_text(&output)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn typed_union_nested_patterns_guards_and_saved_gc_payloads() {
    let source = r#"
type Message do
  Text(String)
  Data(List<Int>, String)
end
fn counted(n :: Int) -> Bool do
  println("guard ${n}")
  n == 2
end
actor reader() do
  send(self(), Text("first-${1}"))
  send(self(), Data([1, 11], "one-${1}"))
  send(self(), Data([2, 22], "two-${2}"))
  send(self(), Text("last-${4}"))
  receive matching do
    Data([n, rest], text) when counted(n) -> println("picked ${text} ${rest}")
  end
  let pressure = for n in 0..2000 do "allocate-${n}" end
  println("allocated ${List.length(pressure)}")
  receive do
    Text(text) -> println(text)
    Data(_, text) -> println(text)
  end
  receive matching do
    Text(text) -> println(text)
  end
  receive do
    Text(text) -> println(text)
    Data(values, text) -> case values do
      [n, rest] -> println("${text} ${n} ${rest}")
      _ -> panic("retained list changed")
    end
  end
end
fn main() do
  let pid :: Pid<Message> = spawn(reader)
end
"#;
    for optimized in [false, true] {
        assert_eq!(
            run(source, optimized),
            "guard 1\nguard 2\npicked two-2 22\nallocated 2000\nfirst-1\nlast-4\none-1 1 11\n"
        );
    }
}

#[test]
fn nested_sum_relocations_survive_saving_guards_and_consumption() {
    let source = r#"
type Inner do
  Payload(String, Int)
end
type Wrapper do
  Wrapped(Inner)
end
type Message do
  Keep(Inner)
  Batch(List<Wrapper>, String)
  Ready(Int)
end
fn reject(text :: String) -> Bool do
  let pressure = for n in 0..2000 do "guard-${n}" end
  println("reject ${text} ${List.length(pressure)}")
  false
end
actor reader() do
  let shared = "shared-${7}"
  send(self(), Keep(Payload("kept-${42}", 42)))
  send(self(), Batch([Wrapped(Payload(shared, 7)), Wrapped(Payload(shared, 8))], "batch-${9}"))
  send(self(), Ready(1))
  receive matching do Ready(n) -> println("ready ${n}") end
  let pressure = for n in 0..2000 do "saved-${n}" end
  println("saved ${List.length(pressure)}")
  receive matching do
    Keep(Payload(text, _)) when reject(text) -> println("unexpected")
    after 0 -> println("retained")
  end
  let inner = receive matching do Keep(value) -> value end
  let accepted_pressure = for n in 0..2000 do "accepted-${n}" end
  println("accepted ${List.length(accepted_pressure)}")
  case inner do Payload(text, n) -> println("${text} ${n}") end
  let items = receive do
    Batch(values, text) -> do
      println(text)
      values
    end
    _ -> panic("expected saved batch")
  end
  let consumed_pressure = for n in 0..2000 do "consumed-${n}" end
  println("consumed ${List.length(consumed_pressure)}")
  for item in items do
    case item do Wrapped(Payload(text, n)) -> println("${text} ${n}") end
  end
end
fn main() do let pid :: Pid<Message> = spawn(reader) end
"#;
    for optimized in [false, true] {
        assert_eq!(
            run(source, optimized),
            "ready 1\nsaved 2000\nreject kept-42 2000\nretained\naccepted 2000\nkept-42 42\nbatch-9\nconsumed 2000\nshared-7 7\nshared-7 8\n"
        );
    }
}

#[test]
fn zero_timeout_and_mixed_receives_retest_once_per_invocation() {
    let source = r#"
fn reject(n :: Int) -> Bool do
  println("tested ${n}")
  false
end
actor reader() do
  send(self(), 1)
  send(self(), 2)
  receive matching do
    n when reject(n) -> println("unexpected")
    after 0 -> println("timeout")
  end
  receive matching do
    n when reject(n) -> println("unexpected")
    after 0 -> println("timeout again")
  end
  receive do n -> println("plain ${n}") end
  receive matching do 2 -> println("matched 2") end
  receive matching do _ -> println("unexpected") after 0 -> println("empty") end
end
fn main() do let pid :: Pid<Int> = spawn(reader) end
"#;
    assert_eq!(run(source, true), "tested 1\ntested 2\ntimeout\ntested 1\ntested 2\ntimeout again\nplain 1\nmatched 2\nempty\n");
}

#[test]
fn guard_can_send_arrival_and_arm_can_receive_with_pending_job_reply() {
    let source = r#"
fn arrived(pid :: Pid<Int>, n :: Int) -> Bool do
  if n == 1 do send(pid, 2) end
  n == 2
end
actor reader() do
  let job = Job.async(fn () -> 42 end)
  send(self(), 1)
  receive matching do
    n when arrived(self(), n) -> do
      println("selected ${n}")
      receive do earlier -> println("earlier ${earlier}") end
    end
  end
  case Job.await(job) do
    Ok(value) -> println("job ${value}")
    Err(reason) -> println(reason)
  end
end
fn main() do let pid :: Pid<Int> = spawn(reader) end
"#;
    assert_eq!(run(source, false), "selected 2\nearlier 1\njob 42\n");
}

#[test]
fn finite_timeout_does_not_retest_saved_messages_on_arrivals() {
    let source = r#"
fn counted(pid :: Pid<Int>, n :: Int) -> Bool do
  println("guard ${n}")
  if n < 4 do Timer.send_after(pid, 1, n + 1) end
  false
end
actor reader(done :: Channel<Int>) do
  send(self(), 1)
  receive matching do
    n when counted(self(), n) -> println("unexpected")
    after 100 -> println("expired")
  end
  receive do n -> println("first ${n}") end
  Channel.try_send(done, 1)
end
fn main() do
  case Channel.bounded(1, :reject_newest) do
    Ok(done) -> do
      let pid :: Pid<Int> = spawn(reader, done)
      case Channel.recv(done, 5_000_000_000) do
        Ok(_) -> 0
        Err(reason) -> panic(reason)
      end
    end
    Err(reason) -> panic(reason)
  end
end
"#;
    assert_eq!(
        run(source, false),
        "guard 1\nguard 2\nguard 3\nguard 4\nexpired\nfirst 1\n"
    );
}

#[test]
fn panic_and_indirect_guard_receive_finalize_actor_and_run_terminate() {
    let source = r#"
fn bad(n :: Int) -> Bool do
  let job = Job.async(fn () -> n end)
  let result = Job.await(job)
  true
end
actor reader() do
  send(self(), 1)
  receive matching do n when bad(n) -> println("unreachable") end
  terminate do println("terminated") end
end
fn main() do let pid :: Pid<Int> = spawn(reader) end
"#;
    assert_eq!(run(source, true), "terminated\n");
    assert_eq!(
        run(
            &source.replace("let result = Job.await(job)", "let result = 10 / (n - n)"),
            false
        ),
        "terminated\n"
    );
}

#[test]
fn saved_messages_apply_backpressure_until_an_arm_consumes_one() {
    let source = r#"
actor reader() do
  for n in 0..1024 do send(self(), n) end
  receive matching do -1 -> println("unexpected") after 0 -> println("saved") end
  println("full ${send(self(), 1024)}")
  receive matching do 1023 -> println("consumed") end
  println("space ${send(self(), 1024)}")
  receive do n -> println("first ${n}") end
end
fn main() do let pid :: Pid<Int> = spawn(reader) end
"#;
    assert_eq!(
        run(source, true),
        "saved\nfull 2\nconsumed\nspace 0\nfirst 0\n"
    );
}

#[test]
fn nonlocal_guard_exit_releases_scan_but_preserves_messages() {
    let source = r#"
actor reader() do
  send(self(), 1)
  for n in 0..1 do
    receive matching do
      value when if value == 1 do break else true end -> println("unexpected")
    end
  end
  receive do value -> println("retained ${value}") end
end
fn main() do let pid :: Pid<Int> = spawn(reader) end
"#;
    assert_eq!(run(source, true), "retained 1\n");
}

#[test]
fn documented_example() {
    assert_eq!(
        run(
            include_str!("../../../examples/selective-receive/main.mpl"),
            false
        ),
        "ready 42\nkept for later\n"
    );
}

#[test]
fn overlapping_or_patterns_run_each_guard_once_in_case_and_receive() {
    let source = r#"
fn reject(n :: Int) -> Bool do
  println("guard ${n}")
  false
end
actor reader() do
  case Some(1) do
    Some(n) | Some(n) when reject(n) -> println("unexpected")
    _ -> println("case fallback")
  end
  send(self(), Some(2))
  receive matching do
    Some(n) | Some(n) when reject(n) -> println("unexpected")
    after 0 -> println("receive timeout")
  end
end
fn main() do let pid :: Pid<Option<Int>> = spawn(reader) end
"#;
    assert_eq!(
        run(source, true),
        "guard 1\ncase fallback\nguard 2\nreceive timeout\n"
    );
}
