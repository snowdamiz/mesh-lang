//! Two Mesh nodes on this machine, connected over the node protocol: what
//! one sends to, spawns on and registers with the other.

#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const COOKIE: &str = "a-development-cookie-0123456789";

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Build `source` as the project `name` under `dir`, with `HUB` and `SPOKE`
/// replaced by the two nodes' names.
fn build(dir: &Path, name: &str, source: &str, nodes: &[(&str, String)]) -> PathBuf {
    let project = dir.join(name);
    std::fs::create_dir_all(&project).unwrap();
    let source = nodes
        .iter()
        .fold(source.replace("COOKIE", COOKIE), |source, (key, node)| {
            source.replace(key, node)
        });
    std::fs::write(project.join("main.mpl"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_meshc"))
        .arg("build")
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "meshc build {name}:\n{}",
        artifacts::command_output_text(&output)
    );
    project.join(name)
}

/// Build both programs, start `hub` and wait for it to print `ready`, then
/// run `spoke` to its end and `hub` to its own. Returns their outputs.
fn run_pair(hub: &str, spoke: &str) -> (String, String) {
    artifacts::ensure_mesh_rt_staticlib();
    let dir = tempfile::tempdir().unwrap();
    let nodes = [
        ("HUB", format!("hub@127.0.0.1:{}", free_port())),
        ("SPOKE", format!("spoke@127.0.0.1:{}", free_port())),
    ];
    let hub = build(dir.path(), "hub", hub, &nodes);
    let spoke = build(dir.path(), "spoke", spoke, &nodes);

    let mut hub = Command::new(hub)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (lines, ready) = mpsc::channel();
    let stdout = BufReader::new(hub.stdout.take().unwrap());
    let reader = std::thread::spawn(move || {
        let mut seen = String::new();
        for line in stdout.lines().map_while(Result::ok) {
            if line == "ready" {
                let _ = lines.send(());
            }
            seen.push_str(&line);
            seen.push('\n');
        }
        seen
    });
    if ready.recv_timeout(artifacts::LAUNCH_ALLOWANCE).is_err() {
        artifacts::stop_child(&mut hub);
        panic!("hub never became ready:\n{}", reader.join().unwrap());
    }

    let spoke = Command::new(spoke)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let spoke = artifacts::wait_with_timeout(spoke, Duration::from_secs(30));
    let hub_status = artifacts::wait_with_timeout(hub, Duration::from_secs(30));
    let hub_stdout = reader.join().unwrap();
    let spoke = spoke.unwrap_or_else(|timed_out| panic!("spoke {timed_out}\nhub:\n{hub_stdout}"));
    let spoke_stdout = artifacts::command_output_text(&spoke);
    let hub = hub_status.unwrap_or_else(|timed_out| {
        panic!("hub {timed_out}\n{hub_stdout}\nspoke:\n{spoke_stdout}")
    });
    assert!(
        hub.status.success() && spoke.status.success(),
        "hub:\n{hub_stdout}{}\nspoke:\n{spoke_stdout}",
        String::from_utf8_lossy(&hub.stderr)
    );
    (
        hub_stdout,
        String::from_utf8_lossy(&spoke.stdout).into_owned(),
    )
}

fn assert_lines(output: &str, expected: &[&str], other: &str) {
    for line in expected {
        assert!(
            output.lines().any(|seen| seen == *line),
            "{line:?} in:\n{output}\nthe other node:\n{other}"
        );
    }
}

/// The hub: under global names, an `echo` actor that answers each number
/// with the next to whoever is registered as `spoke`, and a `greeter` that
/// answers a request at the pid it carries. It ends once the spoke has come
/// and gone.
const HUB: &str = r#"actor echo() do
  receive do
    n -> send(Global.whereis("spoke"), n + 1)
  end
  echo()
end

actor relay(to :: Pid<String>) do
  receive do
    text -> send(to, "relayed " <> text)
  end
end

actor greeter() do
  receive do
    (reply_to, name, tags) -> send(reply_to, "hello #{name}, #{List.length(tags)} tags: #{tags}")
  end
  greeter()
end

fn await_nodes(count :: Int) do
  if List.length(Node.list()) != count do
    Timer.sleep(10)
    await_nodes(count)
  end
end

fn main() do
  let started = Node.start("HUB", "COOKIE")
  let echo :: Pid<Int> = spawn(echo)
  println("register=#{Global.register("echo", echo)}")
  let greeter :: Pid<(Pid<String>, String, List<String>)> = spawn(greeter)
  Global.register("greeter", greeter)
  println("start=#{started}")
  println("ready")
  await_nodes(1)
  await_nodes(0)
  println("spoke_name_gone=#{Global.whereis("spoke") == Process.whereis("none-such")}")
end
"#;

#[test]
fn nodes_exchange_messages_through_global_names() {
    // `Node.connect` returns once the hub's global names are known here.
    let spoke = r#"actor asker() do
  receive do
    _ -> send(Global.whereis("echo"), 41)
  end
  receive do
    n -> println("reply=#{n}")
  end
end

fn await_gone(name :: String) do
  if Global.whereis(name) != Process.whereis("none-such") do
    Timer.sleep(10)
    await_gone(name)
  end
end

fn main() do
  Node.start("SPOKE", "COOKIE")
  println("connect=#{Node.connect("HUB")}")
  println("nodes=#{Node.list()}")
  let asker :: Pid<Int> = spawn(asker)
  println("register=#{Global.register("spoke", asker)}")
  send(asker, 0)
  await_gone("spoke")
end
"#;
    let (hub, spoke) = run_pair(HUB, spoke);
    assert_lines(
        &hub,
        &["start=0", "register=0", "spoke_name_gone=true"],
        &spoke,
    );
    assert_lines(&spoke, &["connect=0", "register=0", "reply=42"], &hub);
    assert!(spoke.contains("nodes=[hub@127.0.0.1:"), "{spoke}");
}

/// A message's strings, literals among them, and lists cross to the other
/// node, sent at once or by a timer, and so does a pid in it, or in a remote
/// spawn's arguments, which the other node can answer at. Code cannot
/// cross: a send of a closure fails.
#[test]
fn nodes_exchange_heap_values_and_answer_the_pids_they_carry() {
    let spoke = r#"actor asker() do
  receive do
    _ -> send(Global.whereis("greeter"), (self(), "spoke", ["red", "green"]))
  end
  receive do
    reply -> println("reply=#{reply}")
  end
  Timer.send_after(Global.whereis("greeter"), 10, (self(), "timer", ["blue"]))
  receive do
    reply -> println("reply=#{reply}")
  end
  let relay = Node.spawn("HUB", relay, self())
  send(relay, "via spawn")
  receive do
    reply -> println("reply=#{reply}")
  end
end

actor relay(to :: Pid<String>) do
  receive do
    text -> send(to, "relayed " <> text)
  end
end

fn await_gone(name :: String) do
  if Global.whereis(name) != Process.whereis("none-such") do
    Timer.sleep(10)
    await_gone(name)
  end
end

fn main() do
  Node.start("SPOKE", "COOKIE")
  Node.connect("HUB")
  println("closure_send=#{send(Global.whereis("echo"), fn -> 1 end)}")
  let asker :: Pid<String> = spawn(asker)
  Global.register("spoke", asker)
  send(asker, "go")
  await_gone("spoke")
end
"#;
    let (hub, spoke) = run_pair(HUB, spoke);
    assert_lines(
        &spoke,
        &[
            "closure_send=6",
            "reply=hello spoke, 2 tags: [red, green]",
            "reply=hello timer, 1 tags: [blue]",
            "reply=relayed via spawn",
        ],
        &hub,
    );
}
