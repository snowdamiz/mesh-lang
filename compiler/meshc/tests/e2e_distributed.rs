//! Two Mesh nodes on this machine, connected over the node protocol: what
//! one sends to, spawns on, registers with, monitors and broadcasts to on
//! the other.

#[path = "support/test_artifacts.rs"]
mod artifacts;
#[path = "support/ws_client.rs"]
mod ws_client;

use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ws_client::TestWsClient;

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

/// The two nodes' names, and the substitutions `build` makes with them.
fn node_names() -> Vec<(&'static str, String)> {
    vec![
        ("HUB", format!("hub@127.0.0.1:{}", free_port())),
        ("SPOKE", format!("spoke@127.0.0.1:{}", free_port())),
    ]
}

/// Start `binary` and wait until it prints `ready`. Returns it and the
/// thread that collects its stdout.
fn start_until_ready(binary: &Path) -> (Child, JoinHandle<String>) {
    let mut child = Command::new(binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (lines, ready) = mpsc::channel();
    let stdout = BufReader::new(child.stdout.take().unwrap());
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
        artifacts::stop_child(&mut child);
        panic!(
            "{} never became ready:\n{}",
            binary.display(),
            reader.join().unwrap()
        );
    }
    (child, reader)
}

/// Run `binary` to its end.
fn run_to_end(binary: &Path) -> Result<Output, artifacts::TimedOut> {
    let child = Command::new(binary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    artifacts::wait_with_timeout(child, Duration::from_secs(30))
}

/// Build both programs, start `hub` and wait for it to print `ready`, then
/// run `spoke` to its end and `hub` to its own. Returns their outputs.
fn run_pair(hub: &str, spoke: &str) -> (String, String) {
    artifacts::ensure_mesh_rt_staticlib();
    let dir = tempfile::tempdir().unwrap();
    let nodes = node_names();
    let hub = build(dir.path(), "hub", hub, &nodes);
    let spoke = build(dir.path(), "spoke", spoke, &nodes);

    let (hub, reader) = start_until_ready(&hub);
    let spoke = run_to_end(&spoke);
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
/// with the next to whoever is registered as `spoke`, a `greeter` that
/// answers a request at the pid it carries, and a `mortal` that ends on its
/// first message. A spoke may spawn a `crasher` on it. It watches the spoke's
/// node, and ends once the spoke has come and gone.
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

actor mortal() do
  receive do
    _ -> println("mortal ends")
  end
end

fn deliberate_crash(0) -> Int do
  0
end

actor crasher() do
  receive do
    n -> println("crasher got #{deliberate_crash(n)}")
  end
end

actor node_watcher() do
  Node.monitor("SPOKE", "spoke node gone")
  receive do
    text -> println(text)
  end
end

fn await_local_gone(name :: String) do
  if Process.whereis(name) != Process.whereis("none-such") do
    Timer.sleep(10)
    await_local_gone(name)
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
  let mortal :: Pid<Int> = spawn(mortal)
  Global.register("mortal", mortal)
  println("start=#{started}")
  println("ready")
  await_nodes(1)
  let watcher :: Pid<String> = spawn(node_watcher)
  Process.register("node_watcher", watcher)
  await_nodes(0)
  await_local_gone("node_watcher")
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

/// A monitor on another node's process fires when it ends, and one removed
/// never does; a process linked to a remote one that crashes ends too; and a
/// node watching another hears when it leaves.
#[test]
fn nodes_monitor_and_link_across_the_connection() {
    let spoke = r#"fn deliberate_crash(0) -> Int do
  0
end

actor crasher() do
  receive do
    n -> println("crasher got #{deliberate_crash(n)}")
  end
end

actor linked() do
  let crasher = Node.spawn_link("HUB", crasher)
  send(crasher, 1)
  receive do
    _ -> println("linked outlived its link")
  end
end

actor watcher() do
  let mortal = Global.whereis("mortal")
  Process.monitor(mortal, "mortal ended")
  send(mortal, 0)
  receive do
    text -> println("got [#{text}]")
  end
  let reference = Process.monitor(Global.whereis("echo"), "echo ended")
  println("remote_demonitor=#{Process.demonitor(reference)}")
  let linked :: Pid<Int> = spawn(linked)
  Process.monitor(linked, "linked ended")
  receive do
    text -> println("got [#{text}]")
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
  let watcher :: Pid<String> = spawn(watcher)
  Global.register("spoke", watcher)
  await_gone("spoke")
end
"#;
    let (hub, spoke) = run_pair(HUB, spoke);
    assert_lines(
        &spoke,
        &[
            "got [mortal ended]",
            "remote_demonitor=0",
            "got [linked ended]",
        ],
        &hub,
    );
    assert!(!spoke.contains("outlived"), "{spoke}");
    assert_lines(&hub, &["mortal ends", "spoke node gone"], &spoke);
}

/// A WebSocket room is cluster-wide: a broadcast on one node reaches the
/// members that joined it on another.
#[test]
fn a_room_broadcast_reaches_members_on_another_node() {
    let hub = r#"fn on_connect(conn, path, headers) do
  Ws.join(conn, "lobby")
  Ws.send(conn, "joined")
  1
end

fn on_message(conn, msg) do
  println(msg)
end

fn on_close(conn, code, reason) do
  println("closed")
end

fn main() do
  Node.start("HUB", "COOKIE")
  println("ready")
  Ws.serve(on_connect, on_message, on_close, WSPORT)
  # Ws.serve returns once it listens; the test stops this node.
  Timer.sleep(600000)
end
"#;
    let spoke = r#"fn main() do
  Node.start("SPOKE", "COOKIE")
  Node.connect("HUB")
  println("broadcast=#{Ws.broadcast("lobby", "from spoke")}")
  # The frame is queued for the session's writer, which the end of main
  # would stop.
  Timer.sleep(1000)
end
"#;
    artifacts::ensure_mesh_rt_staticlib();
    let dir = tempfile::tempdir().unwrap();
    let ws_port = free_port();
    let mut nodes = node_names();
    nodes.push(("WSPORT", ws_port.to_string()));
    let hub = build(dir.path(), "hub", hub, &nodes);
    let spoke = build(dir.path(), "spoke", spoke, &nodes);

    let (mut hub, reader) = start_until_ready(&hub);
    // `ready` comes just before the server starts listening.
    let deadline = Instant::now() + artifacts::LAUNCH_ALLOWANCE;
    let tcp = loop {
        match TcpStream::connect(("127.0.0.1", ws_port)) {
            Ok(tcp) => break tcp,
            Err(error) if Instant::now() > deadline => panic!("hub never listened: {error}"),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    tcp.set_read_timeout(Some(artifacts::LAUNCH_ALLOWANCE + Duration::from_secs(30)))
        .unwrap();
    let mut member = TestWsClient::open(tcp, "/");
    assert_eq!(member.receive_answering_pings(), (1, b"joined".to_vec()));
    // The member keeps answering the server's pings while the spoke starts.
    let member = std::thread::spawn(move || member.receive_answering_pings());

    let spoke = run_to_end(&spoke);
    let received = member.join().unwrap();
    artifacts::stop_child(&mut hub);
    let hub_stdout = reader.join().unwrap();
    let spoke = spoke.unwrap_or_else(|timed_out| panic!("spoke {timed_out}\nhub:\n{hub_stdout}"));
    assert!(
        String::from_utf8_lossy(&spoke.stdout).contains("broadcast=0"),
        "{}",
        artifacts::command_output_text(&spoke)
    );
    assert_eq!(received, (1, b"from spoke".to_vec()), "hub:\n{hub_stdout}");
}
